use std::{
	pin::Pin,
	sync::Arc,
	task::{Context, Poll},
};

use finality_tendermint::{
	messages::{self, GlobalMessageIn, GlobalMessageOut, Precommit, Prevote, Proposal},
	voter, VoterSet,
};
use futures::{channel::mpsc, future, stream, Future, FutureExt, Sink, SinkExt, Stream, StreamExt};
use log::{debug, trace};
use parity_scale_codec::{Decode, Encode};
use parking_lot::Mutex;
use prometheus_endpoint::Registry;
use sc_network::{NetworkService, ReputationChange};
use sc_network_gossip::{GossipEngine, Network as GossipNetwork};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_DEBUG, CONSENSUS_INFO};
use sc_utils::mpsc::TracingUnboundedReceiver;
use sp_finality_tendermint::{AuthorityId, AuthoritySignature, RoundNumber, SetId as SetIdNumber};
use sp_keystore::SyncCryptoStorePtr;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT, NumberFor};

use crate::{
	communication::gossip::VoteMessage, environment::HasVoted, CompactCommit, Error,
	FinalizedCommit, GlobalCommunicationIn, GlobalCommunicationOut, Message, SignedMessage,
};

use self::gossip::{FullCommitMessage, GossipMessage, GossipValidator, PeerReport};

mod gossip;

mod periodic;

/// If the voter set is larger than this value some telemetry events are not
/// sent to avoid increasing usage resource on the node and flooding the
/// telemetry server (e.g. received votes, received precommits.)
const TELEMETRY_VOTERS_LIMIT: usize = 10;

pub mod tendermint_protocol_name {
	use sc_chain_spec::ChainSpec;

	pub(crate) const NAME: &'static str = "/tendermint/1";

	/// Name of the notifications protocol used by GRANDPA.
	///
	/// Must be registered towards the networking in order for GRANDPA to properly function.
	pub fn standard_name<Hash: std::fmt::Display>(
		genesis_hash: &Hash,
		chain_spec: &Box<dyn ChainSpec>,
	) -> std::borrow::Cow<'static, str> {
		let chain_prefix = match chain_spec.fork_id() {
			Some(fork_id) => format!("/{}/{}", genesis_hash, fork_id),
			None => format!("/{}", genesis_hash),
		};
		format!("{}{}", chain_prefix, NAME).into()
	}
}

// cost scalars for reporting peers.
mod cost {
	use sc_network::ReputationChange as Rep;
	pub(super) const PAST_REJECTION: Rep = Rep::new(-50, "Grandpa: Past message");
	pub(super) const BAD_SIGNATURE: Rep = Rep::new(-100, "Grandpa: Bad signature");
	pub(super) const MALFORMED_CATCH_UP: Rep = Rep::new(-1000, "Grandpa: Malformed catch-up");
	pub(super) const MALFORMED_COMMIT: Rep = Rep::new(-1000, "Grandpa: Malformed precommit");
	pub(super) const FUTURE_MESSAGE: Rep = Rep::new(-500, "Grandpa: Future message");
	pub(super) const UNKNOWN_VOTER: Rep = Rep::new(-150, "Grandpa: Unknown voter");

	pub(super) const INVALID_VIEW_CHANGE: Rep = Rep::new(-500, "Grandpa: Invalid round change");
	pub(super) const PER_UNDECODABLE_BYTE: i32 = -5;
	pub(super) const PER_SIGNATURE_CHECKED: i32 = -25;
	pub(super) const PER_BLOCK_LOADED: i32 = -10;
	pub(super) const INVALID_CATCH_UP: Rep = Rep::new(-5000, "Grandpa: Invalid catch-up");
	pub(super) const INVALID_COMMIT: Rep = Rep::new(-5000, "Grandpa: Invalid precommit");
	pub(super) const OUT_OF_SCOPE_MESSAGE: Rep = Rep::new(-500, "Grandpa: Out-of-scope message");
	pub(super) const CATCH_UP_REQUEST_TIMEOUT: Rep =
		Rep::new(-200, "Grandpa: Catch-up request timeout");

	// cost of answering a catch up request
	pub(super) const CATCH_UP_REPLY: Rep = Rep::new(-200, "Grandpa: Catch-up reply");
	pub(super) const HONEST_OUT_OF_SCOPE_CATCH_UP: Rep =
		Rep::new(-200, "Grandpa: Out-of-scope catch-up");
}

// benefit scalars for reporting peers.
mod benefit {
	use sc_network::ReputationChange as Rep;
	pub(super) const NEIGHBOR_MESSAGE: Rep = Rep::new(100, "Grandpa: Neighbor message");
	pub(super) const ROUND_MESSAGE: Rep = Rep::new(100, "Grandpa: Round message");
	pub(super) const BASIC_VALIDATED_CATCH_UP: Rep = Rep::new(200, "Grandpa: Catch-up message");
	pub(super) const BASIC_VALIDATED_COMMIT: Rep = Rep::new(100, "Grandpa: Commit");
	pub(super) const PER_EQUIVOCATION: i32 = 10;
	pub(super) const BASIC_GLOBAL_MESSAGE: Rep = Rep::new(100, "PBFT: Global message");
}
/// A type that ties together our local authority id and a keystore where it is
/// available for signing.
pub struct LocalIdKeystore((AuthorityId, SyncCryptoStorePtr));

impl LocalIdKeystore {
	/// Returns a reference to our local authority id.
	fn local_id(&self) -> &AuthorityId {
		&(self.0).0
	}

	/// Returns a reference to the keystore.
	fn keystore(&self) -> SyncCryptoStorePtr {
		(self.0).1.clone()
	}
}

impl From<(AuthorityId, SyncCryptoStorePtr)> for LocalIdKeystore {
	fn from(inner: (AuthorityId, SyncCryptoStorePtr)) -> LocalIdKeystore {
		LocalIdKeystore(inner)
	}
}

/// A handle to the network.
///
/// Something that provides both the capabilities needed for the `gossip_network::Network` trait as
/// well as the ability to set a fork sync request for a particular block.
pub trait Network<Block: BlockT>: GossipNetwork<Block> + Clone + Send + 'static {
	/// Notifies the sync service to try and sync the given block from the given
	/// peers.
	///
	/// If the given vector of peers is empty then the underlying implementation
	/// should make a best effort to fetch the block from any peers it is
	/// connected to (NOTE: this assumption will change in the future #3629).
	fn set_sync_fork_request(
		&self,
		peers: Vec<sc_network::PeerId>,
		hash: Block::Hash,
		number: NumberFor<Block>,
	);
}

impl<B, H> Network<B> for Arc<NetworkService<B, H>>
where
	B: BlockT,
	H: sc_network::ExHashT,
{
	fn set_sync_fork_request(
		&self,
		peers: Vec<sc_network::PeerId>,
		hash: B::Hash,
		number: NumberFor<B>,
	) {
		NetworkService::set_sync_fork_request(self, peers, hash, number)
	}
}

/// Create a unique topic for a round and set-id combo.
pub(crate) fn round_topic<B: BlockT>(round: RoundNumber, set_id: SetIdNumber) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-{}", set_id, round).as_bytes())
}

/// Create a unique topic for global messages on a set ID.
pub(crate) fn global_topic<B: BlockT>(set_id: SetIdNumber) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-GLOBAL", set_id).as_bytes())
}

/// Bridge between the underlying network service, gossiping consensus messages and Grandpa
#[derive(Clone)]
pub(crate) struct NetworkBridge<B: BlockT, N: Network<B>> {
	service: N,
	gossip_engine: Arc<Mutex<GossipEngine<B>>>,
	validator: Arc<GossipValidator<B>>,

	/// Sender side of the neighbor packet channel.
	///
	/// Packets sent into this channel are processed by the `NeighborPacketWorker` and passed on to
	/// the underlying `GossipEngine`.
	neighbor_sender: periodic::NeighborPacketSender<B>,

	/// `NeighborPacketWorker` processing packets sent through the `NeighborPacketSender`.
	// `NetworkBridge` is required to be cloneable, thus one needs to be able to clone its
	// children, thus one has to wrap `neighbor_packet_worker` with an `Arc` `Mutex`.
	neighbor_packet_worker: Arc<Mutex<periodic::NeighborPacketWorker<B>>>,

	/// Receiver side of the peer report stream populated by the gossip validator, forwarded to the
	/// gossip engine.
	// `NetworkBridge` is required to be cloneable, thus one needs to be able to clone its
	// children, thus one has to wrap gossip_validator_report_stream with an `Arc` `Mutex`. Given
	// that it is just an `UnboundedReceiver`, one could also switch to a
	// multi-producer-*multi*-consumer channel implementation.
	gossip_validator_report_stream: Arc<Mutex<TracingUnboundedReceiver<PeerReport>>>,

	telemetry: Option<TelemetryHandle>,
}

impl<B: BlockT, N: Network<B>> Unpin for NetworkBridge<B, N> {}

impl<B: BlockT, N: Network<B>> NetworkBridge<B, N> {
	/// Create a new network bridge.
	pub fn new(
		service: N,
		config: crate::Config,
		set_state: crate::environment::SharedVoterSetState<B>,
		prometheus_registry: Option<&Registry>,
		telemetry: Option<TelemetryHandle>,
	) -> Self {
		// Get Protocol name.
		let protocal = config.protocol_name.clone();
		// Create GossipValidator.
		let (validator, report_stream) =
			GossipValidator::new(config, set_state.clone(), prometheus_registry, telemetry.clone());

		let validator = Arc::new(validator);

		// Create GossipEngine.
		let gossip_engine = Arc::new(Mutex::new(GossipEngine::new(
			service.clone(),
			protocal,
			validator.clone(),
			prometheus_registry,
		)));

		// TODO: modify this, FKY.
		{
			// register all previous votes with the gossip service so that they're
			// available to peers potentially stuck on a previous round.
			let completed = set_state.read().completed_rounds();
			let (set_id, voters) = completed.set_info();
			validator.note_set(SetId(set_id), voters.to_vec(), |_, _| {});
			for round in completed.iter() {
				let topic = round_topic::<B>(round.number, set_id);

				// we need to note the round with the gossip validator otherwise
				// messages will be ignored.
				validator.note_round(Round(round.number), |_, _| {});

				for signed in round.votes.iter() {
					let message = gossip::GossipMessage::Vote(gossip::VoteMessage::<B> {
						message: signed.clone().into(),
						round: Round(round.number),
						set_id: SetId(set_id),
					});

					gossip_engine.lock().register_gossip_message(topic, message.encode());
				}

				trace!(target: "afp",
					"Registered {} messages for topic {:?} (round: {}, set_id: {})",
					round.votes.len(),
					topic,
					round.number,
					set_id,
				);
			}
		}

		let (neighbor_packet_worker, neighbor_packet_sender) =
			periodic::NeighborPacketWorker::new();

		NetworkBridge {
			service,
			gossip_engine,
			validator,
			neighbor_sender: neighbor_packet_sender,
			neighbor_packet_worker: Arc::new(Mutex::new(neighbor_packet_worker)),
			gossip_validator_report_stream: Arc::new(Mutex::new(report_stream)),
			telemetry,
		}
	}
	/// Note the beginning of a new round to the `GossipValidator`.
	pub(crate) fn note_round(&self, round: Round, set_id: SetId, voters: &VoterSet<AuthorityId>) {
		// is a no-op if currently in that set.
		self.validator.note_set(
			set_id,
			voters.iter().map(|v| v.clone()).collect(),
			|to, neighbor| self.neighbor_sender.send(to, neighbor),
		);

		self.validator
			.note_round(round, |to, neighbor| self.neighbor_sender.send(to, neighbor));
	}

	/// Get a stream of signature-checked round messages from the network as well as a sink for
	/// round messages to the network all within the current set.
	pub(crate) fn round_communication(
		&self,
		keystore: Option<LocalIdKeystore>,
		round: Round,
		set_id: SetId,
		voters: Arc<VoterSet<AuthorityId>>,
		has_voted: HasVoted<B>,
	) -> (impl Stream<Item = SignedMessage<B>> + Unpin, OutgoingMessages<B>) {
		self.note_round(round, set_id, &*voters);

		let keystore = keystore.and_then(|ks| {
			let id = ks.local_id();
			if voters.contains(id) {
				Some(ks)
			} else {
				None
			}
		});

		let topic = round_topic::<B>(round.0, set_id.0);
		let telemetry = self.telemetry.clone();
		let incoming =
			self.gossip_engine.lock().messages_for(topic).filter_map(move |notification| {
				let decoded = GossipMessage::<B>::decode(&mut &notification.message[..]);

				match decoded {
					Err(ref e) => {
						debug!(target: "afp", "Skipping malformed message {:?}: {}", notification, e);
						future::ready(None)
					},
					Ok(GossipMessage::Vote(msg)) => {
						// check signature.
						if !voters.contains(&msg.message.id) {
							debug!(target: "afp", "Skipping message from unknown voter {}", msg.message.id);
							return future::ready(None)
						}

						if voters.len().get() <= TELEMETRY_VOTERS_LIMIT {
							match &msg.message.message {
								messages::Message::Proposal(proposal) => {
									telemetry!(
										telemetry;
										CONSENSUS_INFO;
										"afp.received_propose";
										"voter" => ?format!("{}", msg.message.id),
										"target_height" => ?proposal.target_height,
										"target_hash" => ?proposal.target_hash,
									);
								},
								messages::Message::Prevote(prevote) => {
									telemetry!(
										telemetry;
										CONSENSUS_INFO;
										"afp.received_prevote";
										"voter" => ?format!("{}", msg.message.id),
										"target_height" => ?prevote.target_height,
										"target_hash" => ?prevote.target_hash,
									);
								},
								messages::Message::Precommit(precommit) => {
									telemetry!(
										telemetry;
										CONSENSUS_INFO;
										"afp.received_preprecommit";
										"voter" => ?format!("{}", msg.message.id),
										"target_height" => ?precommit.target_height,
										"target_hash" => ?precommit.target_hash,
									);
								},
							};
						}

						future::ready(Some(msg.message))
					},
					_ => {
						debug!(target: "afp", "Skipping unknown message type");
						future::ready(None)
					},
				}
			});

		let (tx, out_rx) = mpsc::channel(0);
		let outgoing = OutgoingMessages::<B> {
			keystore,
			round: round.0,
			set_id: set_id.0,
			network: self.gossip_engine.clone(),
			sender: tx,
			has_voted,
			telemetry: self.telemetry.clone(),
		};

		// Combine incoming votes from external GRANDPA nodes with outgoing
		// votes from our own GRANDPA voter to have a single
		// vote-import-pipeline.
		let incoming = stream::select(incoming, out_rx);

		(incoming, outgoing)
	}

	/// Set up the global communication streams.
	pub(crate) fn global_communication(
		&self,
		set_id: SetId,
		voters: Arc<VoterSet<AuthorityId>>,
		is_voter: bool,
	) -> (
		impl Stream<Item = GlobalCommunicationIn<B>>,
		impl Sink<GlobalCommunicationOut<B>, Error = Error> + Unpin,
	) {
		self.validator.note_set(
			set_id,
			voters.iter().map(|v| v.clone()).collect(),
			|to, neighbor| self.neighbor_sender.send(to, neighbor),
		);

		let topic = global_topic::<B>(set_id.0);
		log::debug!(target: "afp", "Global topic for incoming_global: {}", topic);
		let incoming = incoming_global(
			self.gossip_engine.clone(),
			topic,
			voters,
			self.validator.clone(),
			self.neighbor_sender.clone(),
			self.telemetry.clone(),
		);

		let outgoing = GlobalMessagesOut::<B>::new(
			self.gossip_engine.clone(),
			set_id.0,
			is_voter,
			self.validator.clone(),
			self.neighbor_sender.clone(),
			self.telemetry.clone(),
		);

		(incoming, outgoing)
	}

	/// Notifies the sync service to try and sync the given block from the given
	/// peers.
	///
	/// If the given vector of peers is empty then the underlying implementation
	/// should make a best effort to fetch the block from any peers it is
	/// connected to (NOTE: this assumption will change in the future #3629).
	pub(crate) fn set_sync_fork_request(
		&self,
		peers: Vec<sc_network::PeerId>,
		hash: B::Hash,
		number: NumberFor<B>,
	) {
		Network::set_sync_fork_request(&self.service, peers, hash, number)
	}
}

impl<B: BlockT, N: Network<B>> Future for NetworkBridge<B, N> {
	type Output = Result<(), Error>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		loop {
			match self.neighbor_packet_worker.lock().poll_next_unpin(cx) {
				Poll::Ready(Some((to, packet))) => {
					self.gossip_engine.lock().send_message(to, packet.encode());
				},
				Poll::Ready(None) =>
					return Poll::Ready(Err(Error::Network(
						"Neighbor packet worker stream closed.".into(),
					))),
				Poll::Pending => break,
			}
		}

		loop {
			match self.gossip_validator_report_stream.lock().poll_next_unpin(cx) {
				Poll::Ready(Some(PeerReport { who, cost_benefit })) => {
					self.gossip_engine.lock().report(who, cost_benefit);
				},
				Poll::Ready(None) =>
					return Poll::Ready(Err(Error::Network(
						"Gossip validator report stream closed.".into(),
					))),
				Poll::Pending => break,
			}
		}

		match self.gossip_engine.lock().poll_unpin(cx) {
			Poll::Ready(()) =>
				return Poll::Ready(Err(Error::Network("Gossip engine future finished.".into()))),
			Poll::Pending => {},
		}

		Poll::Pending
	}
}

/// FIXME: FKY this is a piece of really important code (I guess).
fn incoming_global<B: BlockT>(
	gossip_engine: Arc<Mutex<GossipEngine<B>>>,
	topic: B::Hash,
	voters: Arc<VoterSet<AuthorityId>>,
	gossip_validator: Arc<GossipValidator<B>>,
	neighbor_sender: periodic::NeighborPacketSender<B>,
	telemetry: Option<TelemetryHandle>,
) -> impl Stream<Item = GlobalCommunicationIn<B>> {
	let process_precommit = {
		let telemetry = telemetry.clone();
		move |msg: FullCommitMessage<B>,
		      mut notification: sc_network_gossip::TopicNotification,
		      gossip_engine: &Arc<Mutex<GossipEngine<B>>>,
		      gossip_validator: &Arc<GossipValidator<B>>,
		      voters: &VoterSet<AuthorityId>| {
			if voters.len().get() <= TELEMETRY_VOTERS_LIMIT {
				let preprecommits_signed_by: Vec<String> =
					msg.message.auth_data.iter().map(move |(_, a)| format!("{}", a)).collect();

				telemetry!(
					telemetry;
					CONSENSUS_INFO;
					"afp.received_precommit";
					"contains_preprecommits_signed_by" => ?preprecommits_signed_by,
					"target_height" => ?msg.message.target_number.clone(),
					"target_hash" => ?msg.message.target_hash.clone(),
				);
			}

			if let Err(cost) = check_compact_precommit::<B>(
				&msg.message,
				voters,
				msg.round,
				msg.set_id,
				telemetry.as_ref(),
			) {
				if let Some(who) = notification.sender {
					gossip_engine.lock().report(who, cost);
				}

				return None
			}

			let round = msg.round;
			let set_id = msg.set_id;
			let precommit = msg.message;
			let finalized_number = precommit.target_number;
			let gossip_validator = gossip_validator.clone();
			let gossip_engine = gossip_engine.clone();
			let neighbor_sender = neighbor_sender.clone();
			let cb = move |outcome| match outcome {
				messages::CommitProcessingOutcome::Good(_) => {
					// if it checks out, gossip it. not accounting for
					// any discrepancy between the actual ghost and the claimed
					// finalized number.
					gossip_validator.note_commit_finalized(
						round,
						set_id,
						finalized_number,
						|to, neighbor| neighbor_sender.send(to, neighbor),
					);

					gossip_engine.lock().gossip_message(topic, notification.message.clone(), false);
				},
				messages::CommitProcessingOutcome::Bad(_) => {
					// report peer and do not gossip.
					if let Some(who) = notification.sender.take() {
						gossip_engine.lock().report(who, cost::INVALID_COMMIT);
					}
				},
			};

			let cb = messages::Callback::Work(Box::new(cb));

			Some(GlobalMessageIn::Commit(round.0, precommit, cb))
		}
	};

	// let process_catch_up = move |msg: FullCatchUpMessage<B>,
	//                              mut notification: sc_network_gossip::TopicNotification,
	//                              gossip_engine: &Arc<Mutex<GossipEngine<B>>>,
	//                              gossip_validator: &Arc<GossipValidator<B>>,
	//                              voters: &VoterSet<AuthorityId>| {
	// 	let gossip_validator = gossip_validator.clone();
	// 	let gossip_engine = gossip_engine.clone();
	//
	// 	log::debug!(target: "afp", "process_catch_up");
	//
	// 	if let Err(cost) = check_catch_up::<B>(&msg.message, voters, msg.set_id, telemetry.clone())
	// 	{
	// 		if let Some(who) = notification.sender {
	// 			gossip_engine.lock().report(who, cost);
	// 		}
	//
	// 		return None
	// 	}
	//
	// 	let cb = move |outcome| {
	// 		if let voter::CatchUpProcessingOutcome::Bad(_) = outcome {
	// 			// report peer
	// 			if let Some(who) = notification.sender.take() {
	// 				gossip_engine.lock().report(who, cost::INVALID_CATCH_UP);
	// 			}
	// 		}
	//
	// 		gossip_validator.note_catch_up_message_processed();
	// 	};
	//
	// 	let cb = voter::Callback::Work(Box::new(cb));
	//
	// 	Some(voter::GlobalMessageIn::CatchUp(msg.message, cb))
	// };

	gossip_engine
		.clone()
		.lock()
		.messages_for(topic)
		.filter_map(|notification| {
			debug!(target: "afp", "get global message [filter_map 1]");
			// this could be optimized by decoding piecewise.
			let decoded = GossipMessage::<B>::decode(&mut &notification.message[..]);
			if let Err(ref e) = decoded {
				trace!(target: "afp", "Skipping malformed precommit message {:?}: {}", notification, e);
			}
			future::ready(decoded.map(move |d| (notification, d)).ok())
		})
		.filter_map(move |(notification, msg)| {
			debug!(target: "afp", "get global message [filter_map 2]");
			future::ready(match msg {
				GossipMessage::Commit(msg) => process_precommit(
					msg,
					notification,
					&gossip_engine,
					&gossip_validator,
					&*voters,
				),
				// GossipMessage::CatchUp(msg) =>
				// 	process_catch_up(msg, notification, &gossip_engine, &gossip_validator, &*voters),
				GossipMessage::Global(msg) => match msg.message {
					// crate::GlobalMessage::RoundChange(vc) =>
					// Some(GlobalMessageIn::RoundChange(vc)),
					crate::GlobalMessage::Empty => Some(GlobalMessageIn::Empty),
				},
				_ => {
					// TODO: FKY
					debug!(target: "afp", "Skipping unknown message type");
					None
				},
			})
		})
}

/// Type-safe wrapper around a round number.
#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Encode, Decode)]
pub struct Round(pub RoundNumber);

/// Type-safe wrapper around a set ID.
#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Encode, Decode)]
pub struct SetId(pub SetIdNumber);

/// A sink for outgoing messages to the network. Any messages that are sent will
/// be replaced, as appropriate, according to the given `HasVoted`.
/// NOTE: The votes are stored unsigned, which means that the signatures need to
/// be "stable", i.e. we should end up with the exact same signed message if we
/// use the same raw message and key to sign. This is currently true for
/// `ed25519` and `BLS` signatures (which we might use in the future), care must
/// be taken when switching to different key types.
pub(crate) struct OutgoingMessages<Block: BlockT> {
	round: RoundNumber,
	set_id: SetIdNumber,
	keystore: Option<LocalIdKeystore>,
	sender: mpsc::Sender<SignedMessage<Block>>,
	network: Arc<Mutex<GossipEngine<Block>>>,
	has_voted: HasVoted<Block>,
	telemetry: Option<TelemetryHandle>,
}

impl<B: BlockT> Unpin for OutgoingMessages<B> {}

impl<Block: BlockT> Sink<Message<Block>> for OutgoingMessages<Block> {
	type Error = Error;

	fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Sink::poll_ready(Pin::new(&mut self.sender), cx).map(|elem| {
			elem.map_err(|e| {
				Error::Network(format!("Failed to poll_ready channel sender: {:?}", e))
			})
		})
	}

	fn start_send(mut self: Pin<&mut Self>, mut msg: Message<Block>) -> Result<(), Self::Error> {
		// if we've voted on this round previously under the same key, send that vote instead
		match &mut msg {
			messages::Message::Proposal(ref mut vote) => {
				if let Some(pre_prevote) = self.has_voted.proposal() {
					*vote = pre_prevote.clone();
				}
			},
			messages::Message::Prevote(ref mut vote) =>
				if let Some(prevote) = self.has_voted.prevote() {
					*vote = prevote.clone();
				},
			messages::Message::Precommit(ref mut vote) =>
				if let Some(precommit) = self.has_voted.precommit() {
					*vote = precommit.clone();
				},
		}

		// when locals exist, sign messages on import
		if let Some(ref keystore) = self.keystore {
			// QUESTION: FKY how does grandpa deal with it if no new block generated.
			// TODO:     It should still gossip message's, but what contained in it's target_hash
			// and      target_height?
			let target_hash = msg.target().0.clone();
			let signed = sp_finality_tendermint::sign_message(
				keystore.keystore(),
				msg,
				keystore.local_id().clone(),
				self.round,
				self.set_id,
			)
			.ok_or_else(|| {
				Error::Signing(format!(
					"Failed to sign GRANDPA vote for round {} targetting {:?}",
					self.round, target_hash
				))
			})?;

			let message = GossipMessage::Vote(VoteMessage::<Block> {
				message: signed.clone(),
				round: Round(self.round),
				set_id: SetId(self.set_id),
			});

			debug!(
				target: "afp",
				"Announcing block {:?} to peers which we voted on in round {} in set {}",
				target_hash,
				self.round,
				self.set_id,
			);

			telemetry!(
				self.telemetry;
				CONSENSUS_DEBUG;
				"afp.announcing_blocks_to_voted_peers";
				"block" => ?target_hash, "round" => ?self.round, "set_id" => ?self.set_id,
			);

			if let Some(target_hash) = target_hash {
				// announce the block we voted on to our peers.
				self.network.lock().announce(target_hash, None);
			}
			// propagate the message to peers
			let topic = round_topic::<Block>(self.round, self.set_id);
			self.network.lock().gossip_message(topic, message.encode(), false);

			// forward the message to the inner sender.
			return self.sender.start_send(signed).map_err(|e| {
				Error::Network(format!("Failed to start_send on channel sender: {:?}", e))
			})
		};

		Ok(())
	}

	fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Sink::poll_close(Pin::new(&mut self.sender), cx).map(|elem| {
			elem.map_err(|e| {
				Error::Network(format!("Failed to poll_close channel sender: {:?}", e))
			})
		})
	}
}

// checks a compact precommit. returns the cost associated with processing it if
// the precommit was bad.
fn check_compact_precommit<Block: BlockT>(
	msg: &CompactCommit<Block>,
	voters: &VoterSet<AuthorityId>,
	round: Round,
	set_id: SetId,
	telemetry: Option<&TelemetryHandle>,
) -> Result<(), ReputationChange> {
	// TODO: refactor
	// check total len is not out of range.
	if &msg.auth_data.len() > &voters.len().get() {
		return Err(cost::MALFORMED_COMMIT)
	}

	for (_, ref id) in &msg.auth_data {
		if let None = voters.get(id) {
			debug!(target: "afp", "Skipping precommit containing unknown voter {}", id);
			return Err(cost::MALFORMED_COMMIT)
		}
	}

	// Super majority.
	if &msg.auth_data.len() < &voters.threshold() {
		return Err(cost::MALFORMED_COMMIT)
	}

	// check signatures on all contained preprecommits.
	let mut buf = Vec::new();
	for (i, (precommit, &(ref sig, ref id))) in
		msg.commits.iter().zip(&msg.auth_data).enumerate()
	{
		use crate::communication::gossip::Misbehavior;
		use messages::Message as TendermintMessage;

		if !sp_finality_tendermint::check_message_signature_with_buffer(
			&TendermintMessage::Precommit(precommit.clone()),
			id,
			sig,
			round.0,
			set_id.0,
			&mut buf,
		) {
			debug!(target: "afp", "Bad precommit message signature {}", id);
			telemetry!(
				telemetry;
				CONSENSUS_DEBUG;
				"afp.bad_precommit_msg_signature";
				"id" => ?id,
			);
			let cost = Misbehavior::BadCommitMessage {
				signatures_checked: i as i32,
				blocks_loaded: 0,
				equivocations_caught: 0,
			}
			.cost();

			return Err(cost)
		}
	}

	Ok(())
}

// checks a catch up. returns the cost associated with processing it if
// the catch up was bad.
// fn check_catch_up<Block: BlockT>(
// 	msg: &CatchUp<Block>,
// 	voters: &VoterSet<AuthorityId>,
// 	set_id: SetId,
// 	telemetry: Option<TelemetryHandle>,
// ) -> Result<(), ReputationChange> {
// 	let full_len = voters.len().get();
//
// 	fn check_len<'a>(
// 		voters: &VoterSet<AuthorityId>,
// 		threshold: usize,
// 		msgs_len: usize,
// 		msgs: impl Iterator<Item = &'a AuthorityId>,
// 		full_len: usize,
// 	) -> Result<(), ReputationChange> {
// 		// Super majority.
// 		if msgs_len < threshold {
// 			return Err(cost::MALFORMED_CATCH_UP)
// 		}
//
// 		// check total len is not out of range.
// 		if msgs_len > full_len {
// 			return Err(cost::MALFORMED_CATCH_UP)
// 		}
//
// 		for id in msgs {
// 			if let None = voters.get(&id) {
// 				debug!(target: "afp", "Skipping catch up message containing unknown voter {}", id);
// 				return Err(cost::MALFORMED_CATCH_UP)
// 			}
// 		}
//
// 		Ok(())
// 	}
//
// 	// check_len(
// 	// 	voters,
// 	// 	voters.threshold(),
// 	// 	msg.prevotes.len(),
// 	// 	msg.prevotes.iter().map(|vote| &vote.id),
// 	// 	full_len,
// 	// )?;
//
// 	check_len(
// 		voters,
// 		voters.threshold(),
// 		msg.precommits.len(),
// 		msg.precommits.iter().map(|vote| &vote.id),
// 		full_len,
// 	)?;
//
// 	fn check_signatures<'a, B, I>(
// 		messages: I,
// 		round: RoundNumber,
// 		set_id: SetIdNumber,
// 		mut signatures_checked: usize,
// 		buf: &mut Vec<u8>,
// 		telemetry: Option<TelemetryHandle>,
// 	) -> Result<usize, ReputationChange>
// 	where
// 		B: BlockT,
// 		I: Iterator<Item = (Message<B>, &'a AuthorityId, &'a AuthoritySignature)>,
// 	{
// 		use crate::communication::gossip::Misbehavior;
//
// 		for (msg, id, sig) in messages {
// 			signatures_checked += 1;
//
// 			if !sp_finality_tendermint::check_message_signature_with_buffer(
// 				&msg, id, sig, round, set_id, buf,
// 			) {
// 				debug!(target: "afp", "Bad catch up message signature {}", id);
// 				telemetry!(
// 					telemetry;
// 					CONSENSUS_DEBUG;
// 					"afp.bad_catch_up_msg_signature";
// 					"id" => ?id,
// 				);
//
// 				let cost = Misbehavior::BadCatchUpMessage {
// 					signatures_checked: signatures_checked as i32,
// 				}
// 				.cost();
//
// 				return Err(cost)
// 			}
// 		}
//
// 		Ok(signatures_checked)
// 	}
//
// 	let mut buf = Vec::new();
//
// 	// check signatures on all contained prevotes.
// 	let signatures_checked = check_signatures::<Block, _>(
// 		msg.prevotes.iter().map(|vote| {
// 			(leader::Message::Prevote(vote.prevote.clone()), &vote.id, &vote.signature)
// 		}),
// 		msg.round_number,
// 		set_id.0,
// 		0,
// 		&mut buf,
// 		telemetry.clone(),
// 	)?;
//
// 	// check signatures on all contained preprecommits.
// 	let _ = check_signatures::<Block, _>(
// 		msg.precommits.iter().map(|vote| {
// 			(leader::Message::Precommit(vote.precommit.clone()), &vote.id, &vote.signature)
// 		}),
// 		msg.round_number,
// 		set_id.0,
// 		signatures_checked,
// 		&mut buf,
// 		telemetry,
// 	)?;
//
// 	Ok(())
// }

/// An output sink for precommit messages.
struct GlobalMessagesOut<Block: BlockT> {
	network: Arc<Mutex<GossipEngine<Block>>>,
	set_id: SetId,
	is_voter: bool,
	gossip_validator: Arc<GossipValidator<Block>>,
	neighbor_sender: periodic::NeighborPacketSender<Block>,
	telemetry: Option<TelemetryHandle>,
}

impl<Block: BlockT> GlobalMessagesOut<Block> {
	/// Create a new precommit output stream.
	pub(crate) fn new(
		network: Arc<Mutex<GossipEngine<Block>>>,
		set_id: SetIdNumber,
		is_voter: bool,
		gossip_validator: Arc<GossipValidator<Block>>,
		neighbor_sender: periodic::NeighborPacketSender<Block>,
		telemetry: Option<TelemetryHandle>,
	) -> Self {
		log::debug!(target: "afp", "GlobalMessagesOut::new is_voter: {}", is_voter);
		GlobalMessagesOut {
			network,
			set_id: SetId(set_id),
			is_voter,
			gossip_validator,
			neighbor_sender,
			telemetry,
		}
	}
}

// FIXME: use GlobalMessageOut instead of FinalizedCommit
// Because ChangeRound and Empty also should be delivered without delayed.
impl<Block: BlockT> Sink<GlobalCommunicationOut<Block>> for GlobalMessagesOut<Block> {
	type Error = Error;

	fn poll_ready(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn start_send(
		self: Pin<&mut Self>,
		input: GlobalCommunicationOut<Block>,
	) -> Result<(), Self::Error> {
		if !self.is_voter {
			return Ok(())
		}

		let message = match input {
			GlobalMessageOut::Commit(round, f_commit) => {
				let round = Round(round);

				telemetry!(
					self.telemetry;
					CONSENSUS_DEBUG;
					"afp.global_message";
					"target_height" => ?f_commit.target_number,
					"target_hash" => ?f_commit.target_hash,
				);
				let (commits, auth_data) = f_commit
					.commits
					.into_iter()
					.map(|signed| (signed.commit, (signed.signature, signed.id)))
					.unzip();

				let compact_precommit = CompactCommit::<Block> {
					target_hash: f_commit.target_hash,
					target_number: f_commit.target_number,
					commits,
					auth_data,
				};

				let message = GossipMessage::Commit(FullCommitMessage::<Block> {
					round,
					set_id: self.set_id,
					message: compact_precommit,
				});

				// the gossip validator needs to be made aware of the best precommit-height we know
				// of before gossiping
				self.gossip_validator.note_commit_finalized(
					round,
					self.set_id,
					f_commit.target_number,
					|to, neighbor| self.neighbor_sender.send(to, neighbor),
				);

				message
			},
			// GlobalMessageOut::RoundChange(round_change) =>
			// 	GossipMessage::Global(gossip::GlobalMessage {
			// 		set_id: self.set_id,
			// 		message: crate::GlobalMessage::RoundChange(round_change),
			// 	}),
			GlobalMessageOut::Empty => GossipMessage::Global(gossip::GlobalMessage {
				set_id: self.set_id,
				message: crate::GlobalMessage::Empty,
			}),
		};

		let topic = global_topic::<Block>(self.set_id.0);

		log::debug!(target: "afp", "set_id: {:?}, topic: {:?}, global message: {:?}",self.set_id, topic ,message, );

		self.network.lock().gossip_message(topic, message.encode(), false);

		Ok(())
	}

	fn poll_close(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn poll_flush(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}
}
