use std::{
	collections::{HashSet, VecDeque},
	time::{Duration, Instant},
};

use ahash::{AHashMap, AHashSet};
use log::{debug, trace};
use parity_scale_codec::{Decode, Encode};
use prometheus_endpoint::{register, CounterVec, Opts, PrometheusError, Registry, U64};
use rand::prelude::SliceRandom;
use sc_network::{ObservedRole, PeerId, ReputationChange};
use sc_network_gossip::{MessageIntent, ValidatorContext};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_DEBUG};
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver, TracingUnboundedSender};
use sp_api::NumberFor;
use sp_arithmetic::traits::Zero;
use sp_finality_tendermint::AuthorityId;
use sp_runtime::traits::Block as BlockT;

use crate::{environment, CompactCommit, SignedMessage};

use super::{benefit, cost, Round, SetId};

const REBROADCAST_AFTER: Duration = Duration::from_secs(60 * 5);
const CATCH_UP_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const CATCH_UP_PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum number of rounds we are behind a peer before issuing a
/// catch up request.
const CATCH_UP_THRESHOLD: u64 = 0;

/// The total round duration measured in periods of gossip duration:
/// 2 gossip durations for prevote timer
/// 2 gossip durations for precommit timer
/// 1 gossip duration for precommits to spread
const ROUND_DURATION: u32 = 5;

/// The period, measured in rounds, since the latest round start, after which we will start
/// propagating gossip messages to more nodes than just the lucky ones.
const PROPAGATION_SOME: f32 = 1.5;

/// The period, measured in rounds, since the latest round start, after which we will start
/// propagating gossip messages to all the nodes we are connected to.
const PROPAGATION_ALL: f32 = 3.0;

/// Assuming a network of 3000 nodes, using a fanout of 4, after about 6 iterations
/// of gossip a message has very likely reached all nodes on the network (`log4(3000)`).
const LUCKY_PEERS: usize = 4;

type Report = (PeerId, ReputationChange);

/// An outcome of examining a message.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Consider {
	/// Accept the message.
	Accept,
	/// Message is too early. Reject.
	RejectPast,
	/// Message is from the future. Reject.
	RejectFuture,
	/// Message cannot be evaluated. Reject.
	RejectOutOfScope,
}

/// A round of protocal state.
#[derive(Debug)]
struct PeerRound<N> {
	round: Round,           // the current round we are at.
	set_id: SetId,          // the current voter set id.
	last_commit: Option<N>, // commit-finalized block height, if any.
}

impl<N> Default for PeerRound<N> {
	fn default() -> Self {
		PeerRound { round: Round(0), set_id: SetId(0), last_commit: None }
	}
}

impl<N: Ord> PeerRound<N> {
	/// Consider a round and set ID combination under a current round.
	fn consider_vote(&self, round: Round, set_id: SetId) -> Consider {
		// only from current set
		if set_id < self.set_id {
			return Consider::RejectPast
		}
		if set_id > self.set_id {
			return Consider::RejectFuture
		}

		// only r-1 ... r+1
		if round.0 > self.round.0.saturating_add(1) {
			return Consider::RejectFuture
		}
		if round.0 < self.round.0.saturating_sub(1) {
			return Consider::RejectPast
		}

		Consider::Accept
	}

	/// Consider a set-id global message. Rounds are not taken into account, but are implicitly
	/// because we gate on finalization of a further block than a previous commit.
	fn consider_global(&self, set_id: SetId, number: N) -> Consider {
		// only from current set
		if set_id < self.set_id {
			return Consider::RejectPast
		}
		if set_id > self.set_id {
			return Consider::RejectFuture
		}

		// only commits which claim to prove a higher block number than
		// the one we're aware of.
		match self.last_commit {
			None => Consider::Accept,
			Some(ref num) =>
				if num < &number {
					Consider::Accept
				} else {
					Consider::RejectPast
				},
		}
	}
}

/// A local round of protocol state. Similar to `Round` but we additionally track
/// the round and set id at which the last commit was observed, and the instant
/// at which the current round started.
struct LocalRound<N> {
	round: Round,
	set_id: SetId,
	last_commit: Option<(N, Round, SetId)>,
	round_start: Instant,
}

impl<N> LocalRound<N> {
	/// Creates a new `LocalRound` at the given set id and round.
	fn new(set_id: SetId, round: Round) -> LocalRound<N> {
		LocalRound { set_id, round, last_commit: None, round_start: Instant::now() }
	}

	/// Converts the local round to a `Round` discarding round and set id
	/// information about the last commit.
	fn as_round(&self) -> PeerRound<&N> {
		PeerRound { round: self.round, set_id: self.set_id, last_commit: self.last_commit_height() }
	}

	/// Update the set ID. implies a reset to round 1.
	fn update_set(&mut self, set_id: SetId) {
		if set_id != self.set_id {
			self.set_id = set_id;
			self.round = Round(1);
			self.round_start = Instant::now();
		}
	}

	/// Updates the current round.
	fn update_round(&mut self, round: Round) {
		self.round = round;
		self.round_start = Instant::now();
	}

	/// Returns the height of the block that the last observed commit finalizes.
	fn last_commit_height(&self) -> Option<&N> {
		self.last_commit.as_ref().map(|(number, _, _)| number)
	}
}

const KEEP_RECENT_ROUNDS: usize = 3;

/// Tracks gossip topics that we are keeping messages for. We keep topics of:
///
/// - the last `KEEP_RECENT_ROUNDS` complete GRANDPA rounds,
///
/// - the topic for the current and next round,
///
/// - and a global topic for commit and catch-up messages.
struct KeepTopics<B: BlockT> {
	current_set: SetId,
	rounds: VecDeque<(Round, SetId)>,
	reverse_map: AHashMap<B::Hash, (Option<Round>, SetId)>,
}

impl<B: BlockT> KeepTopics<B> {
	fn new() -> Self {
		KeepTopics {
			current_set: SetId(0),
			rounds: VecDeque::with_capacity(KEEP_RECENT_ROUNDS + 2),
			reverse_map: Default::default(),
		}
	}

	fn push(&mut self, round: Round, set_id: SetId) {
		self.current_set = std::cmp::max(self.current_set, set_id);

		// under normal operation the given round is already tracked (since we
		// track one round ahead). if we skip rounds (with a catch up) the given
		// round topic might not be tracked yet.
		if !self.rounds.contains(&(round, set_id)) {
			self.rounds.push_back((round, set_id));
		}

		// we also accept messages for the next round
		self.rounds.push_back((Round(round.0.saturating_add(1)), set_id));

		// the 2 is for the current and next round.
		while self.rounds.len() > KEEP_RECENT_ROUNDS + 2 {
			let _ = self.rounds.pop_front();
		}

		let mut map = AHashMap::with_capacity(KEEP_RECENT_ROUNDS + 3);
		map.insert(super::global_topic::<B>(self.current_set.0), (None, self.current_set));

		for &(round, set) in &self.rounds {
			map.insert(super::round_topic::<B>(round.0, set.0), (Some(round), set));
		}

		self.reverse_map = map;
	}

	fn topic_info(&self, topic: &B::Hash) -> Option<(Option<Round>, SetId)> {
		self.reverse_map.get(topic).cloned()
	}
}

// topics to send to a neighbor based on their round.
fn neighbor_topics<B: BlockT>(peer_round: &PeerRound<NumberFor<B>>) -> Vec<B::Hash> {
	let s = peer_round.set_id;
	let mut topics =
		vec![super::global_topic::<B>(s.0), super::round_topic::<B>(peer_round.round.0, s.0)];

	if peer_round.round.0 != 0 {
		let r = Round(peer_round.round.0 - 1);
		topics.push(super::round_topic::<B>(r.0, s.0))
	}

	topics
}

/// Grandpa gossip message type.
/// This is the root type that gets encoded and sent on the network.
#[derive(Debug, Encode, Decode)]
pub(super) enum GossipMessage<Block: BlockT> {
	/// PBFT message with round and set info.
	Vote(VoteMessage<Block>),
	/// PBFT commit message with round and set info.
	Commit(FullCommitMessage<Block>),
	/// PBFT global message
	Global(GlobalMessage),
	/// A neighbor packet. Not repropagated.
	Neighbor(VersionedNeighborPacket<NumberFor<Block>>),
	// Grandpa catch up request message with round and set info. Not repropagated.
	// CatchUpRequest(CatchUpRequestMessage),
	// Grandpa catch up message with round and set info. Not repropagated.
	// CatchUp(FullCatchUpMessage<Block>),
}

impl<Block: BlockT> From<NeighborPacket<NumberFor<Block>>> for GossipMessage<Block> {
	fn from(neighbor: NeighborPacket<NumberFor<Block>>) -> Self {
		GossipMessage::Neighbor(VersionedNeighborPacket::V1(neighbor))
	}
}

/// Network level message with topic information.
#[derive(Debug, Encode, Decode)]
pub(super) struct VoteMessage<Block: BlockT> {
	/// The round this message is from.
	pub(super) round: Round,
	/// The voter set ID this message is from.
	pub(super) set_id: SetId,
	/// The message itself.
	pub(super) message: SignedMessage<Block>,
}

/// Network level commit message with topic information.
#[derive(Debug, Encode, Decode)]
pub(super) struct FullCommitMessage<Block: BlockT> {
	/// The round this message is from.
	pub(super) round: Round,
	/// The voter set ID this message is from.
	pub(super) set_id: SetId,
	/// The compact commit message.
	pub(super) message: CompactCommit<Block>,
}

/// Network level global message
#[derive(Debug, Encode, Decode)]
pub(super) struct GlobalMessage {
	/// The voter set ID this message is from.
	pub(super) set_id: SetId,
	/// The global message.
	pub(super) message: crate::GlobalMessage,
}

/// V1 neighbor packet. Neighbor packets are sent from nodes to their peers
/// and are not repropagated. These contain information about the node's state.
#[derive(Debug, Encode, Decode, Clone)]
pub(super) struct NeighborPacket<N> {
	/// The round the node is currently at.
	pub(super) round: Round,
	/// The set ID the node is currently at.
	pub(super) set_id: SetId,
	/// The highest finalizing commit observed.
	pub(super) commit_finalized_height: N,
}

/// A versioned neighbor packet.
#[derive(Debug, Encode, Decode)]
pub(super) enum VersionedNeighborPacket<N> {
	#[codec(index = 1)]
	V1(NeighborPacket<N>),
}

impl<N> VersionedNeighborPacket<N> {
	fn into_neighbor_packet(self) -> NeighborPacket<N> {
		match self {
			VersionedNeighborPacket::V1(p) => p,
		}
	}
}

// A catch up request for a given round (or any further round) localized by set id.
// #[derive(Clone, Debug, Encode, Decode)]
// pub(super) struct CatchUpRequestMessage {
// 	/// The round that we want to catch up to.
// 	pub(super) round: Round,
// 	/// The voter set ID this message is from.
// 	pub(super) set_id: SetId,
// }

// Network level catch up message with topic information.
// #[derive(Debug, Encode, Decode)]
// pub(super) struct FullCatchUpMessage<Block: BlockT> {
// 	/// The voter set ID this message is from.
// 	pub(super) set_id: SetId,
// 	/// The compact commit message.
// 	pub(super) message: CatchUp<Block>,
// }

/// Misbehavior that peers can perform.
///
/// `cost` gives a cost that can be used to perform cost/benefit analysis of a
/// peer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Misbehavior {
	// invalid neighbor message, considering the last one.
	InvalidRoundChange,
	// could not decode neighbor message. bytes-length of the packet.
	UndecodablePacket(i32),
	// Bad catch up message (invalid signatures).
	// BadCatchUpMessage { signatures_checked: i32 },
	// Bad commit message
	BadCommitMessage { signatures_checked: i32, blocks_loaded: i32, equivocations_caught: i32 },
	// A message received that's from the future relative to our round.
	// always misbehavior.
	FutureMessage,
	// A message received that cannot be evaluated relative to our round.
	// This happens before we have a round and have sent out neighbor packets.
	// always misbehavior.
	OutOfScopeMessage,
}

impl Misbehavior {
	pub(super) fn cost(&self) -> ReputationChange {
		use Misbehavior::*;

		match *self {
			InvalidRoundChange => cost::INVALID_VIEW_CHANGE,
			UndecodablePacket(bytes) => ReputationChange::new(
				bytes.saturating_mul(cost::PER_UNDECODABLE_BYTE),
				"Grandpa: Bad packet",
			),
			// BadCatchUpMessage { signatures_checked } => ReputationChange::new(
			// 	cost::PER_SIGNATURE_CHECKED.saturating_mul(signatures_checked),
			// 	"Grandpa: Bad cath-up message",
			// ),
			BadCommitMessage { signatures_checked, blocks_loaded, equivocations_caught } => {
				let cost = cost::PER_SIGNATURE_CHECKED
					.saturating_mul(signatures_checked)
					.saturating_add(cost::PER_BLOCK_LOADED.saturating_mul(blocks_loaded));

				let benefit = equivocations_caught.saturating_mul(benefit::PER_EQUIVOCATION);

				ReputationChange::new(
					(benefit as i32).saturating_add(cost as i32),
					"Grandpa: Bad commit",
				)
			},
			FutureMessage => cost::FUTURE_MESSAGE,
			OutOfScopeMessage => cost::OUT_OF_SCOPE_MESSAGE,
		}
	}
}

#[derive(Debug)]
struct PeerInfo<N> {
	round: PeerRound<N>,
	roles: ObservedRole,
}

impl<N> PeerInfo<N> {
	fn new(roles: ObservedRole) -> Self {
		PeerInfo { round: PeerRound::default(), roles }
	}
}

/// The peers we're connected to in gossip.
struct Peers<N> {
	inner: AHashMap<PeerId, PeerInfo<N>>,
	/// The randomly picked set of `LUCKY_PEERS` we'll gossip to in the first stage of round
	/// gossiping.
	first_stage_peers: AHashSet<PeerId>,
	/// The randomly picked set of peers we'll gossip to in the second stage of gossiping if
	/// the first stage didn't allow us to spread the voting data enough to conclude the round.
	/// This set should have size `sqrt(connected_peers)`.
	second_stage_peers: HashSet<PeerId>,
	/// The randomly picked set of `LUCKY_PEERS` light clients we'll gossip commit messages to.
	lucky_light_peers: HashSet<PeerId>,
}

impl<N> Default for Peers<N> {
	fn default() -> Self {
		Peers {
			inner: Default::default(),
			first_stage_peers: Default::default(),
			second_stage_peers: Default::default(),
			lucky_light_peers: Default::default(),
		}
	}
}

impl<N: Ord + std::fmt::Debug> Peers<N> {
	fn new_peer(&mut self, who: PeerId, role: ObservedRole) {
		match role {
			ObservedRole::Authority if self.first_stage_peers.len() < LUCKY_PEERS => {
				self.first_stage_peers.insert(who.clone());
			},
			ObservedRole::Authority if self.second_stage_peers.len() < LUCKY_PEERS => {
				self.second_stage_peers.insert(who.clone());
			},
			ObservedRole::Light if self.lucky_light_peers.len() < LUCKY_PEERS => {
				self.lucky_light_peers.insert(who.clone());
			},
			_ => {},
		}

		self.inner.insert(who, PeerInfo::new(role));
	}

	fn peer_disconnected(&mut self, who: &PeerId) {
		self.inner.remove(who);
		// This does not happen often enough compared to round duration,
		// so we don't reshuffle.
		self.first_stage_peers.remove(who);
		self.second_stage_peers.remove(who);
		self.lucky_light_peers.remove(who);
	}

	// returns a reference to the new round, if the peer is known.
	fn update_peer_state(
		&mut self,
		who: &PeerId,
		update: NeighborPacket<N>,
	) -> Result<Option<&PeerRound<N>>, Misbehavior> {
		let peer = match self.inner.get_mut(who) {
			None => return Ok(None),
			Some(p) => p,
		};

		log::debug!(target: "afp", "Received neighbor packet from {:?}, update: {:?}, peer: {:?}", who, update, peer);

		let invalid_change = peer.round.set_id > update.set_id ||
			peer.round.round > update.round && peer.round.set_id == update.set_id ||
			peer.round.last_commit.as_ref() > Some(&update.commit_finalized_height);

		if invalid_change {
			return Err(Misbehavior::InvalidRoundChange)
		}

		peer.round = PeerRound {
			round: update.round,
			set_id: update.set_id,
			last_commit: Some(update.commit_finalized_height),
		};

		trace!(target: "afp", "Peer {} updated round. Now at {:?}, {:?}",
			who, peer.round.round, peer.round.set_id);

		Ok(Some(&peer.round))
	}

	fn update_commit_height(&mut self, who: &PeerId, new_height: N) -> Result<(), Misbehavior> {
		let peer = match self.inner.get_mut(who) {
			None => return Ok(()),
			Some(p) => p,
		};

		// this doesn't allow a peer to send us unlimited commits with the
		// same height, because there is still a misbehavior condition based on
		// sending commits that are <= the best we are aware of.
		if peer.round.last_commit.as_ref() > Some(&new_height) {
			return Err(Misbehavior::InvalidRoundChange)
		}

		peer.round.last_commit = Some(new_height);

		Ok(())
	}

	fn peer<'a>(&'a self, who: &PeerId) -> Option<&'a PeerInfo<N>> {
		self.inner.get(who)
	}

	fn reshuffle(&mut self) {
		// we want to randomly select peers into three sets according to the following logic:
		// - first set: LUCKY_PEERS random peers where at least LUCKY_PEERS/2 are authorities
		//   (unless
		// we're not connected to that many authorities)
		// - second set: max(LUCKY_PEERS, sqrt(peers)) peers where at least LUCKY_PEERS are
		//   authorities.
		// - third set: LUCKY_PEERS random light client peers

		let shuffled_peers = {
			let mut peers = self
				.inner
				.iter()
				.map(|(peer_id, info)| (*peer_id, info.clone()))
				.collect::<Vec<_>>();

			peers.shuffle(&mut rand::thread_rng());
			peers
		};

		let shuffled_authorities = shuffled_peers.iter().filter_map(|(peer_id, info)| {
			if matches!(info.roles, ObservedRole::Authority) {
				Some(peer_id)
			} else {
				None
			}
		});

		let mut first_stage_peers = AHashSet::new();
		let mut second_stage_peers = HashSet::new();

		// we start by allocating authorities to the first stage set and when the minimum of
		// `LUCKY_PEERS / 2` is filled we start allocating to the second stage set.
		let half_lucky = LUCKY_PEERS / 2;
		let one_and_a_half_lucky = LUCKY_PEERS + half_lucky;
		for (n_authorities_added, peer_id) in shuffled_authorities.enumerate() {
			if n_authorities_added < half_lucky {
				first_stage_peers.insert(*peer_id);
			} else if n_authorities_added < one_and_a_half_lucky {
				second_stage_peers.insert(*peer_id);
			} else {
				break
			}
		}

		// fill up first and second sets with remaining peers (either full or authorities)
		// prioritizing filling the first set over the second.
		let n_second_stage_peers = LUCKY_PEERS.max((shuffled_peers.len() as f32).sqrt() as usize);
		for (peer_id, info) in &shuffled_peers {
			if info.roles.is_light() {
				continue
			}

			if first_stage_peers.len() < LUCKY_PEERS {
				first_stage_peers.insert(*peer_id);
				second_stage_peers.remove(peer_id);
			} else if second_stage_peers.len() < n_second_stage_peers {
				if !first_stage_peers.contains(peer_id) {
					second_stage_peers.insert(*peer_id);
				}
			} else {
				break
			}
		}

		// pick `LUCKY_PEERS` random light peers
		let lucky_light_peers = shuffled_peers
			.into_iter()
			.filter_map(|(peer_id, info)| if info.roles.is_light() { Some(peer_id) } else { None })
			.take(LUCKY_PEERS)
			.collect();

		self.first_stage_peers = first_stage_peers;
		self.second_stage_peers = second_stage_peers;
		self.lucky_light_peers = lucky_light_peers;
	}
}

#[derive(Debug, PartialEq)]
pub(super) enum Action<H> {
	// repropagate under given topic, to the given peers, applying cost/benefit to originator.
	Keep(H, ReputationChange),
	// discard and process.
	ProcessAndDiscard(H, ReputationChange),
	// discard, applying cost/benefit to originator.
	Discard(ReputationChange),
}

/// State of catch up request handling.
#[derive(Debug)]
enum PendingCatchUp {
	/// No pending catch up requests.
	None,
	// /// Pending catch up request which has not been answered yet.
	// Requesting { who: PeerId, request: CatchUpRequestMessage, instant: Instant },
	/// Pending catch up request that was answered and is being processed.
	Processing { instant: Instant },
}

/// Configuration for the round catch-up mechanism.
enum CatchUpConfig {
	/// Catch requests are enabled, our node will issue them whenever it sees a
	/// neighbor packet for a round further than `CATCH_UP_THRESHOLD`. If
	/// `only_from_authorities` is set, the node will only send catch-up
	/// requests to other authorities it is connected to. This is useful if the
	/// GRANDPA observer protocol is live on the network, in which case full
	/// nodes (non-authorities) don't have the necessary round data to answer
	/// catch-up requests.
	Enabled { only_from_authorities: bool },
	/// Catch-up requests are disabled, our node will never issue them. This is
	/// useful for the GRANDPA observer mode, where we are only interested in
	/// commit messages and don't need to follow the full round protocol.
	Disabled,
}

impl CatchUpConfig {
	fn enabled(only_from_authorities: bool) -> CatchUpConfig {
		CatchUpConfig::Enabled { only_from_authorities }
	}

	fn disabled() -> CatchUpConfig {
		CatchUpConfig::Disabled
	}

	fn request_allowed<N>(&self, peer: &PeerInfo<N>) -> bool {
		match self {
			CatchUpConfig::Disabled => false,
			CatchUpConfig::Enabled { only_from_authorities, .. } => match peer.roles {
				ObservedRole::Authority => true,
				ObservedRole::Light => false,
				ObservedRole::Full => !only_from_authorities,
			},
		}
	}
}

struct Inner<Block: BlockT> {
	local_round: Option<LocalRound<NumberFor<Block>>>,
	peers: Peers<NumberFor<Block>>,
	live_topics: KeepTopics<Block>,
	authorities: Vec<AuthorityId>,
	config: crate::Config,
	next_rebroadcast: Instant,
	pending_catch_up: PendingCatchUp,
	catch_up_config: CatchUpConfig,
}

type MaybeMessage<Block> = Option<(Vec<PeerId>, NeighborPacket<NumberFor<Block>>)>;

impl<Block: BlockT> Inner<Block> {
	fn new(config: crate::Config) -> Self {
		let catch_up_config = CatchUpConfig::enabled(true);

		Inner {
			local_round: None,
			peers: Peers::default(),
			live_topics: KeepTopics::new(),
			next_rebroadcast: Instant::now() + REBROADCAST_AFTER,
			authorities: Vec::new(),
			pending_catch_up: PendingCatchUp::None,
			catch_up_config,
			config,
		}
	}

	/// Note a round in the current set has started.
	fn note_round(&mut self, round: Round) -> MaybeMessage<Block> {
		{
			let local_round = match self.local_round {
				None => return None,
				Some(ref mut v) =>
					if v.round == round {
						return None
					} else {
						v
					},
			};

			let set_id = local_round.set_id;

			debug!(target: "afp", "Voter {} noting beginning of round {:?} to network.",
				self.config.name(), (round, set_id));

			local_round.update_round(round);

			self.live_topics.push(round, set_id);
			self.peers.reshuffle();
		}
		self.multicast_neighbor_packet()
	}

	/// Note that a voter set with given ID has started. Does nothing if the last
	/// call to the function was with the same `set_id`.
	fn note_set(&mut self, set_id: SetId, authorities: Vec<AuthorityId>) -> MaybeMessage<Block> {
		{
			let local_round = match self.local_round {
				ref mut x @ None => x.get_or_insert(LocalRound::new(set_id, Round(1))),
				Some(ref mut v) =>
					if v.set_id == set_id {
						let diff_authorities = self.authorities.iter().collect::<HashSet<_>>() !=
							authorities.iter().collect();

						if diff_authorities {
							debug!(target: "afp",
								"Gossip validator noted set {:?} twice with different authorities. \
								Was the authority set hard forked?",
								set_id,
							);
							self.authorities = authorities;
						}
						return None
					} else {
						v
					},
			};

			local_round.update_set(set_id);
			self.live_topics.push(Round(1), set_id);
			self.authorities = authorities;
		}
		self.multicast_neighbor_packet()
	}

	/// Note that we've imported a commit finalizing a given block.
	fn note_commit_finalized(
		&mut self,
		round: Round,
		set_id: SetId,
		finalized: NumberFor<Block>,
	) -> MaybeMessage<Block> {
		{
			match self.local_round {
				None => return None,
				Some(ref mut v) =>
					if v.last_commit_height() < Some(&finalized) {
						v.last_commit = Some((finalized, round, set_id));
					} else {
						return None
					},
			};
		}

		self.multicast_neighbor_packet()
	}

	fn consider_vote(&self, round: Round, set_id: SetId) -> Consider {
		self.local_round
			.as_ref()
			.map(LocalRound::as_round)
			.map(|v| v.consider_vote(round, set_id))
			.unwrap_or(Consider::RejectOutOfScope)
	}

	fn consider_global(&self, set_id: SetId, number: NumberFor<Block>) -> Consider {
		self.local_round
			.as_ref()
			.map(LocalRound::as_round)
			.map(|v| v.consider_global(set_id, &number))
			.unwrap_or(Consider::RejectOutOfScope)
	}

	fn cost_past_rejection(
		&self,
		_who: &PeerId,
		_round: Round,
		_set_id: SetId,
	) -> ReputationChange {
		// hardcoded for now.
		cost::PAST_REJECTION
	}

	fn validate_round_message(
		&self,
		who: &PeerId,
		full: &VoteMessage<Block>,
	) -> Action<Block::Hash> {
		match self.consider_vote(full.round, full.set_id) {
			Consider::RejectFuture => return Action::Discard(Misbehavior::FutureMessage.cost()),
			Consider::RejectOutOfScope =>
				return Action::Discard(Misbehavior::OutOfScopeMessage.cost()),
			Consider::RejectPast =>
				return Action::Discard(self.cost_past_rejection(who, full.round, full.set_id)),
			Consider::Accept => {},
		}

		// ensure authority is part of the set.
		if !self.authorities.contains(&full.message.id) {
			debug!(target: "afp", "Message from unknown voter: {}", full.message.id);
			telemetry!(
				self.config.telemetry;
				CONSENSUS_DEBUG;
				"afp.bad_msg_signature";
				"signature" => ?full.message.id,
			);
			return Action::Discard(cost::UNKNOWN_VOTER)
		}

		if !sp_finality_tendermint::check_message_signature(
			&full.message.message,
			&full.message.id,
			&full.message.signature,
			full.round.0,
			full.set_id.0,
		) {
			debug!(target: "afp", "Bad message signature {}", full.message.id);
			telemetry!(
				self.config.telemetry;
				CONSENSUS_DEBUG;
				"afp.bad_msg_signature";
				"signature" => ?full.message.id,
			);
			return Action::Discard(cost::BAD_SIGNATURE)
		}

		let topic = super::round_topic::<Block>(full.round.0, full.set_id.0);
		Action::Keep(topic, benefit::ROUND_MESSAGE)
	}

	fn validate_commit_message(
		&mut self,
		who: &PeerId,
		full: &FullCommitMessage<Block>,
	) -> Action<Block::Hash> {
		if let Err(misbehavior) = self.peers.update_commit_height(who, full.message.target_number) {
			return Action::Discard(misbehavior.cost())
		}

		match self.consider_global(full.set_id, full.message.target_number) {
			Consider::RejectFuture => return Action::Discard(Misbehavior::FutureMessage.cost()),
			Consider::RejectPast =>
				return Action::Discard(self.cost_past_rejection(who, full.round, full.set_id)),
			Consider::RejectOutOfScope =>
				return Action::Discard(Misbehavior::OutOfScopeMessage.cost()),
			Consider::Accept => {},
		}

		if full.message.commits.len() != full.message.auth_data.len() ||
			full.message.commits.is_empty()
		{
			debug!(target: "afp", "Malformed compact commit");
			telemetry!(
				self.config.telemetry;
				CONSENSUS_DEBUG;
				"afp.malformed_compact_commit";
				"commits_len" => ?full.message.commits.len(),
				"auth_data_len" => ?full.message.auth_data.len(),
				"commits_is_empty" => ?full.message.commits.is_empty(),
			);
			return Action::Discard(cost::MALFORMED_COMMIT)
		}

		// always discard commits initially and rebroadcast after doing full
		// checking.
		let topic = super::global_topic::<Block>(full.set_id.0);
		Action::ProcessAndDiscard(topic, benefit::BASIC_VALIDATED_COMMIT)
	}

	fn validate_global_message(
		&mut self,
		who: &PeerId,
		full: &GlobalMessage,
	) -> Action<Block::Hash> {
		// always discard catch up messages, they're point-to-point
		log::debug!(target: "afp", "validate_global_message: {:?}", full);
		let topic = super::global_topic::<Block>(full.set_id.0);
		return Action::ProcessAndDiscard(topic, benefit::NEIGHBOR_MESSAGE)
	}

	// fn validate_catch_up_message(
	// 	&mut self,
	// 	who: &PeerId,
	// 	full: &FullCatchUpMessage<Block>,
	// ) -> Action<Block::Hash> {
	// 	match &self.pending_catch_up {
	// 		PendingCatchUp::Requesting { who: peer, request, instant } => {
	// 			if peer != who {
	// 				return Action::Discard(Misbehavior::OutOfScopeMessage.cost())
	// 			}
	//
	// 			if request.set_id != full.set_id {
	// 				return Action::Discard(cost::MALFORMED_CATCH_UP)
	// 			}
	//
	// 			log::trace!(target: "afp", "full: {:?}", full.message);
	// 			// if request.round.0 > full.message.round_number {
	// 			//                 log::trace!(target: "afp", "full: {:?}", full.message);
	// 			// 	return Action::Discard(cost::MALFORMED_CATCH_UP)
	// 			// }
	//
	// 			if full.message.commits.is_empty() {
	// 				return Action::Discard(cost::MALFORMED_CATCH_UP)
	// 			}
	//
	// 			// move request to pending processing state, we won't push out
	// 			// any catch up requests until we import this one (either with a
	// 			// success or failure).
	// 			self.pending_catch_up = PendingCatchUp::Processing { instant: *instant };
	//
	// 			// always discard catch up messages, they're point-to-point
	// 			let topic = super::global_topic::<Block>(full.set_id.0);
	// 			Action::ProcessAndDiscard(topic, benefit::BASIC_VALIDATED_CATCH_UP)
	// 		},
	// 		_ => Action::Discard(Misbehavior::OutOfScopeMessage.cost()),
	// 	}
	// }

	fn note_catch_up_message_processed(&mut self) {
		match &self.pending_catch_up {
			PendingCatchUp::Processing { .. } => {
				self.pending_catch_up = PendingCatchUp::None;
			},
			state => debug!(target: "afp",
				"Noted processed catch up message when state was: {:?}",
				state,
			),
		}
	}

	// fn handle_catch_up_request(
	// 	&mut self,
	// 	who: &PeerId,
	// 	request: CatchUpRequestMessage,
	// 	set_state: &environment::SharedVoterSetState<Block>,
	// ) -> (Option<GossipMessage<Block>>, Action<Block::Hash>) {
	// 	trace!(target: "afp", "handle_catch_up_request");
	// 	let local_round = match self.local_round {
	// 		None => return (None, Action::Discard(Misbehavior::OutOfScopeMessage.cost())),
	// 		Some(ref round) => round,
	// 	};
	//
	// 	if request.set_id != local_round.set_id {
	// 		// NOTE: When we're close to a set change there is potentially a
	// 		// race where the peer sent us the request before it observed that
	// 		// we had transitioned to a new set. In this case we charge a lower
	// 		// cost.
	// 		if request.set_id.0.saturating_add(1) == local_round.set_id.0 &&
	// 			local_round.round.0.saturating_sub(CATCH_UP_THRESHOLD) == 0
	// 		{
	// 			return (None, Action::Discard(cost::HONEST_OUT_OF_SCOPE_CATCH_UP))
	// 		}
	//
	// 		return (None, Action::Discard(Misbehavior::OutOfScopeMessage.cost()))
	// 	}
	// 	trace!(target: "afp", "handle_catch_up_request phase 2");
	//
	// 	match self.peers.peer(who) {
	// 		None => return (None, Action::Discard(Misbehavior::OutOfScopeMessage.cost())),
	// 		// NOTE: we change >= to > because when round_change,
	// 		// a lost-behind node may in round 0, and we are in round 1.
	// 		Some(peer) if peer.round.round > request.round => {
	// 			trace!(target: "afp", "handle_catch_up_request phase {:?} {:?}", peer.round.round,
	// request.round); 			return (None, Action::Discard(Misbehavior::OutOfScopeMessage.cost()))
	// 		},
	// 		_ => {},
	// 	}
	// 	trace!(target: "afp", "handle_catch_up_request phase 3");
	//
	// 	let last_completed_round = set_state.read().last_completed_round();
	// 	if last_completed_round.number < request.round.0 {
	// 		return (None, Action::Discard(Misbehavior::OutOfScopeMessage.cost()))
	// 	}
	//
	// 	trace!(target: "afp", "Replying to catch-up request for round {} from {} with round {}",
	// 		request.round.0,
	// 		who,
	// 		last_completed_round.number,
	// 	);
	//
	// 	// TODO: remove this.
	// 	let mut prepares = Vec::new();
	//
	// 	let mut commits = Vec::new();
	//
	// 	trace!(target: "afp", "handle_catch_up_request votes {:?}", last_completed_round.votes);
	//
	// 	// NOTE: the set of votes stored in `LastCompletedRound` is a minimal
	// 	// set of votes, i.e. at most one equivocation is stored per voter. The
	// 	// code below assumes this invariant is maintained when creating the
	// 	// catch up reply since peers won't accept catch-up messages that have
	// 	// too many equivocations (we exceed the fault-tolerance bound).
	// 	commits = last_completed_round.votes.clone();
	//
	// 	let (base_number, base_hash) = last_completed_round.base;
	//
	// 	let catch_up = CatchUp::<Block> {
	// 		round_number: last_completed_round.number,
	// 		prepares,
	// 		commits,
	// 		base_hash,
	// 		base_number,
	// 	};
	//
	// 	let full_catch_up = GossipMessage::CatchUp::<Block>(FullCatchUpMessage {
	// 		set_id: request.set_id,
	// 		message: catch_up,
	// 	});
	//
	// 	(Some(full_catch_up), Action::Discard(cost::CATCH_UP_REPLY))
	// }
	//
	// fn try_catch_up(&mut self, who: &PeerId) -> (Option<GossipMessage<Block>>, Option<Report>) {
	// 	log::trace!(target: "afp", "try_catch_up from {:?}", who);
	// 	let mut catch_up = None;
	// 	let mut report = None;
	//
	// 	// if the peer is on the same set and ahead of us by a margin bigger
	// 	// than `CATCH_UP_THRESHOLD` then we should ask it for a catch up
	// 	// message. we only send catch-up requests to authorities, observers
	// 	// won't be able to reply since they don't follow the full GRANDPA
	// 	// protocol and therefore might not have the vote data available.
	// 	if let (Some(peer), Some(local_round)) = (self.peers.peer(who), &self.local_round) {
	// 		log::trace!(target: "afp", "try_catch_up peer: {:?}, local_round: {:?}", peer,
	// local_round.round); 		log::trace!(target: "afp", "try_catch_up {} [{:?} {:?}] [{:?} {:?}]",
	// self.catch_up_config.request_allowed(&peer), peer.round.set_id, local_round.set_id,
	// peer.round.round.0.saturating_sub(CATCH_UP_THRESHOLD), local_round.round.0 );
	// 		if self.catch_up_config.request_allowed(&peer) &&
	// 			peer.round.set_id == local_round.set_id &&
	// 			peer.round.round.0.saturating_sub(CATCH_UP_THRESHOLD) > local_round.round.0
	// 		{
	// 			// send catch up request if allowed
	// 			let round = peer.round.round.0 - 1; // peer.round.round is > 0
	// 			let request = CatchUpRequestMessage { set_id: peer.round.set_id, round: Round(round) };
	//
	// 			log::trace!(target: "afp", "try_catch_up sending request: {:?}", request);
	//
	// 			let (catch_up_allowed, catch_up_report) = self.note_catch_up_request(who, &request);
	//
	// 			if catch_up_allowed {
	// 				debug!(target: "afp", "Sending catch-up request for round {} to {}",
	// 					   round,
	// 					   who,
	// 				);
	//
	// 				catch_up = Some(GossipMessage::<Block>::CatchUpRequest(request));
	// 			}
	//
	// 			report = catch_up_report;
	// 		}
	// 	}
	//
	// 	log::trace!(target: "afp", "try_catch_up end with catch_up: {:?}", catch_up);
	// 	(catch_up, report)
	// }

	fn import_neighbor_message(
		&mut self,
		who: &PeerId,
		update: NeighborPacket<NumberFor<Block>>,
	) -> (Vec<Block::Hash>, Action<Block::Hash>)
// Option<GossipMessage<Block>>, Option<Report>
	{
		let update_res = self.peers.update_peer_state(who, update);

		let (cost_benefit, topics) = match update_res {
			Ok(round) =>
				(benefit::NEIGHBOR_MESSAGE, round.map(|round| neighbor_topics::<Block>(round))),
			Err(misbehavior) => (misbehavior.cost(), None),
		};

		// let (catch_up, report) = match update_res {
		// 	Ok(_) => self.try_catch_up(who),
		// 	_ => (None, None),
		// };

		let neighbor_topics = topics.unwrap_or_default();

		// always discard neighbor messages, it's only valid for one hop.
		let action = Action::Discard(cost_benefit);

		(neighbor_topics, action)
	}

	fn multicast_neighbor_packet(&self) -> MaybeMessage<Block> {
		self.local_round.as_ref().map(|local_round| {
			let packet = NeighborPacket {
				round: local_round.round,
				set_id: local_round.set_id,
				commit_finalized_height: *local_round.last_commit_height().unwrap_or(&Zero::zero()),
			};

			let peers = self
				.peers
				.inner
				.iter()
				.filter_map(|(id, info)| {
					// light clients don't participate in the full GRANDPA voter protocol
					// and therefore don't need to be informed about round updates
					if info.roles.is_light() {
						None
					} else {
						Some(id)
					}
				})
				.cloned()
				.collect();

			(peers, packet)
		})
	}

	// fn note_catch_up_request(
	// 	&mut self,
	// 	who: &PeerId,
	// 	catch_up_request: &CatchUpRequestMessage,
	// ) -> (bool, Option<Report>) {
	// 	let report = match &self.pending_catch_up {
	// 		PendingCatchUp::Requesting { who: peer, instant, .. } => {
	// 			if instant.elapsed() <= CATCH_UP_REQUEST_TIMEOUT {
	// 				return (false, None)
	// 			} else {
	// 				// report peer for timeout
	// 				Some((peer.clone(), cost::CATCH_UP_REQUEST_TIMEOUT))
	// 			}
	// 		},
	// 		PendingCatchUp::Processing { instant, .. } => {
	// 			if instant.elapsed() < CATCH_UP_PROCESS_TIMEOUT {
	// 				return (false, None)
	// 			} else {
	// 				None
	// 			}
	// 		},
	// 		_ => None,
	// 	};
	//
	// 	self.pending_catch_up = PendingCatchUp::Requesting {
	// 		who: who.clone(),
	// 		request: catch_up_request.clone(),
	// 		instant: Instant::now(),
	// 	};
	//
	// 	(true, report)
	// }

	/// The initial logic for filtering round messages follows the given state
	/// transitions:
	///
	/// - State 1: allowed to LUCKY_PEERS random peers (where at least LUCKY_PEERS/2 are
	///   authorities)
	/// - State 2: allowed to max(LUCKY_PEERS, sqrt(random peers)) (where at least LUCKY_PEERS are
	///   authorities)
	/// - State 3: allowed to all peers
	///
	/// Transitions will be triggered on repropagation attempts by the underlying gossip layer.
	fn round_message_allowed(&self, who: &PeerId) -> bool {
		let round_duration = self.config.gossip_duration * ROUND_DURATION;
		let round_elapsed = match self.local_round {
			Some(ref local_round) => local_round.round_start.elapsed(),
			None => return false,
		};

		if self.config.local_role.is_light() {
			return false
		}

		if round_elapsed < round_duration.mul_f32(PROPAGATION_SOME) {
			self.peers.first_stage_peers.contains(who)
		} else if round_elapsed < round_duration.mul_f32(PROPAGATION_ALL) {
			self.peers.first_stage_peers.contains(who) ||
				self.peers.second_stage_peers.contains(who)
		} else {
			self.peers.peer(who).map(|info| !info.roles.is_light()).unwrap_or(false)
		}
	}

	/// The initial logic for filtering global messages follows the given state
	/// transitions:
	///
	/// - State 1: allowed to max(LUCKY_PEERS, sqrt(peers)) (where at least LUCKY_PEERS are
	///   authorities)
	/// - State 2: allowed to all peers
	///
	/// We are more lenient with global messages since there should be a lot
	/// less global messages than round messages (just commits), and we want
	/// these to propagate to non-authorities fast enough so that they can
	/// observe finality.
	///
	/// Transitions will be triggered on repropagation attempts by the
	/// underlying gossip layer, which should happen every 30 seconds.
	fn global_message_allowed(&self, who: &PeerId) -> bool {
		let round_duration = self.config.gossip_duration * ROUND_DURATION;
		let round_elapsed = match self.local_round {
			Some(ref local_round) => local_round.round_start.elapsed(),
			None => return false,
		};

		if self.config.local_role.is_light() {
			return false
		}

		if round_elapsed < round_duration.mul_f32(PROPAGATION_ALL) {
			self.peers.first_stage_peers.contains(who) ||
				self.peers.second_stage_peers.contains(who) ||
				self.peers.lucky_light_peers.contains(who)
		} else {
			true
		}
	}
}

// Prometheus metrics for [`GossipValidator`].
pub(crate) struct Metrics {
	messages_validated: CounterVec<U64>,
}

impl Metrics {
	pub(crate) fn register(
		registry: &prometheus_endpoint::Registry,
	) -> Result<Self, PrometheusError> {
		Ok(Self {
			messages_validated: register(
				CounterVec::new(
					Opts::new(
						"substrate_finality_grandpa_communication_gossip_validator_messages",
						"Number of messages validated by the finality grandpa gossip validator.",
					),
					&["message", "action"],
				)?,
				registry,
			)?,
		})
	}
}
/// A validator for PBFT gossip messages.
pub(super) struct GossipValidator<Block: BlockT> {
	inner: parking_lot::RwLock<Inner<Block>>,
	set_state: environment::SharedVoterSetState<Block>,
	report_sender: TracingUnboundedSender<PeerReport>,
	metrics: Option<Metrics>,
	telemetry: Option<TelemetryHandle>,
}

impl<Block: BlockT> GossipValidator<Block> {
	/// Create a new gossip-validator. The current set is initialized to 0. If
	/// `catch_up_enabled` is set to false then the validator will not issue any
	/// catch up requests (useful e.g. when running just the GRANDPA observer).
	pub(super) fn new(
		config: crate::Config,
		set_state: environment::SharedVoterSetState<Block>,
		prometheus_registry: Option<&Registry>,
		telemetry: Option<TelemetryHandle>,
	) -> (GossipValidator<Block>, TracingUnboundedReceiver<PeerReport>) {
		let metrics = match prometheus_registry.map(Metrics::register) {
			Some(Ok(metrics)) => Some(metrics),
			Some(Err(e)) => {
				debug!(target: "afp", "Failed to register metrics: {:?}", e);
				None
			},
			None => None,
		};

		let (tx, rx) = tracing_unbounded("mpsc_grandpa_gossip_validator");
		let val = GossipValidator {
			inner: parking_lot::RwLock::new(Inner::new(config)),
			set_state,
			report_sender: tx,
			metrics,
			telemetry,
		};

		(val, rx)
	}

	/// Note a round in the current set has started.
	pub(super) fn note_round<F>(&self, round: Round, send_neighbor: F)
	where
		F: FnOnce(Vec<PeerId>, NeighborPacket<NumberFor<Block>>),
	{
		let maybe_msg = self.inner.write().note_round(round);
		if let Some((to, msg)) = maybe_msg {
			send_neighbor(to, msg);
		}
	}

	/// Note that a voter set with given ID has started. Updates the current set to given
	/// value and initializes the round to 0.
	pub(super) fn note_set<F>(&self, set_id: SetId, authorities: Vec<AuthorityId>, send_neighbor: F)
	where
		F: FnOnce(Vec<PeerId>, NeighborPacket<NumberFor<Block>>),
	{
		let maybe_msg = self.inner.write().note_set(set_id, authorities);
		if let Some((to, msg)) = maybe_msg {
			send_neighbor(to, msg);
		}
	}

	/// Note that we've imported a commit finalizing a given block.
	pub(super) fn note_commit_finalized<F>(
		&self,
		round: Round,
		set_id: SetId,
		finalized: NumberFor<Block>,
		send_neighbor: F,
	) where
		F: FnOnce(Vec<PeerId>, NeighborPacket<NumberFor<Block>>),
	{
		let maybe_msg = self.inner.write().note_commit_finalized(round, set_id, finalized);

		if let Some((to, msg)) = maybe_msg {
			send_neighbor(to, msg);
		}
	}

	/// Note that we've processed a catch up message.
	pub(super) fn note_catch_up_message_processed(&self) {
		self.inner.write().note_catch_up_message_processed();
	}

	fn report(&self, who: PeerId, cost_benefit: ReputationChange) {
		let _ = self.report_sender.unbounded_send(PeerReport { who, cost_benefit });
	}

	pub(super) fn do_validate(
		&self,
		who: &PeerId,
		mut data: &[u8],
	) -> (Action<Block::Hash>, Vec<Block::Hash>, Option<GossipMessage<Block>>) {
		let mut broadcast_topics = Vec::new();
		let mut peer_reply = None;

		// Message name for Prometheus metric recording.
		let message_name;

		let action = {
			match GossipMessage::<Block>::decode(&mut data) {
				Ok(GossipMessage::Vote(ref message)) => {
					message_name = Some("vote");
					self.inner.write().validate_round_message(who, message)
				},
				Ok(GossipMessage::Commit(ref message)) => {
					message_name = Some("commit");
					self.inner.write().validate_commit_message(who, message)
				},
				Ok(GossipMessage::Global(ref message)) => {
					message_name = Some("global");
					self.inner.write().validate_global_message(who, message)
				},
				Ok(GossipMessage::Neighbor(update)) => {
					message_name = Some("neighbor");
					// let (topics, action, catch_up, report) = self
					let (topics, action) = self
						.inner
						.write()
						.import_neighbor_message(who, update.into_neighbor_packet());

					// if let Some((peer, cost_benefit)) = report {
					// 	self.report(peer, cost_benefit);
					// }

					broadcast_topics = topics;
					// peer_reply = catch_up;
					action
				},
				// Ok(GossipMessage::CatchUp(ref message)) => {
				// 	message_name = Some("catch_up");
				// 	self.inner.write().validate_catch_up_message(who, message)
				// },
				// Ok(GossipMessage::CatchUpRequest(request)) => {
				// 	message_name = Some("catch_up_request");
				// 	let (reply, action) =
				// 		self.inner.write().handle_catch_up_request(who, request, &self.set_state);
				//
				// 	peer_reply = reply;
				// 	action
				// },
				Err(e) => {
					message_name = None;
					debug!(target: "afp", "Error decoding message: {}", e);
					telemetry!(
						self.telemetry;
						CONSENSUS_DEBUG;
						"afp.err_decoding_msg";
						"" => "",
					);

					let len = std::cmp::min(i32::MAX as usize, data.len()) as i32;
					Action::Discard(Misbehavior::UndecodablePacket(len).cost())
				},
			}
		};

		// Prometheus metric recording.
		if let (Some(metrics), Some(message_name)) = (&self.metrics, message_name) {
			let action_name = match action {
				Action::Keep(_, _) => "keep",
				Action::ProcessAndDiscard(_, _) => "process_and_discard",
				Action::Discard(_) => "discard",
			};
			metrics.messages_validated.with_label_values(&[message_name, action_name]).inc();
		}

		log::trace!(target: "afp", "do validate, message_name: {:?}, action: {:?}, broadcast_topics: {:?}", message_name, action, broadcast_topics,);

		(action, broadcast_topics, peer_reply)
	}

	#[cfg(test)]
	fn inner(&self) -> &parking_lot::RwLock<Inner<Block>> {
		&self.inner
	}
}

impl<Block: BlockT> sc_network_gossip::Validator<Block> for GossipValidator<Block> {
	fn new_peer(
		&self,
		context: &mut dyn ValidatorContext<Block>,
		who: &PeerId,
		roles: ObservedRole,
	) {
		let packet = {
			let mut inner = self.inner.write();
			inner.peers.new_peer(who.clone(), roles);

			inner.local_round.as_ref().map(|v| NeighborPacket {
				round: v.round,
				set_id: v.set_id,
				commit_finalized_height: *v.last_commit_height().unwrap_or(&Zero::zero()),
			})
		};

		if let Some(packet) = packet {
			let packet_data = GossipMessage::<Block>::from(packet).encode();
			context.send_message(who, packet_data);
		}
	}

	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId) {
		self.inner.write().peers.peer_disconnected(who);
	}

	fn validate(
		&self,
		context: &mut dyn ValidatorContext<Block>,
		who: &PeerId,
		data: &[u8],
	) -> sc_network_gossip::ValidationResult<Block::Hash> {
		let (action, broadcast_topics, peer_reply) = self.do_validate(who, data);

		// not with lock held!
		if let Some(msg) = peer_reply {
			context.send_message(who, msg.encode());
		}

		for topic in broadcast_topics {
			context.send_topic(who, topic, false);
		}

		match action {
			Action::Keep(topic, cb) => {
				self.report(who.clone(), cb);
				context.broadcast_message(topic, data.to_vec(), false);
				sc_network_gossip::ValidationResult::ProcessAndKeep(topic)
			},
			Action::ProcessAndDiscard(topic, cb) => {
				self.report(who.clone(), cb);
				sc_network_gossip::ValidationResult::ProcessAndDiscard(topic)
			},
			Action::Discard(cb) => {
				self.report(who.clone(), cb);
				sc_network_gossip::ValidationResult::Discard
			},
		}
	}

	fn message_allowed<'a>(
		&'a self,
	) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a> {
		let (inner, do_rebroadcast) = {
			use parking_lot::RwLockWriteGuard;

			let mut inner = self.inner.write();
			let now = Instant::now();
			let do_rebroadcast = if now >= inner.next_rebroadcast {
				inner.next_rebroadcast = now + REBROADCAST_AFTER;
				true
			} else {
				false
			};

			// downgrade to read-lock.
			(RwLockWriteGuard::downgrade(inner), do_rebroadcast)
		};

		Box::new(move |who, intent, topic, mut data| {
			if let MessageIntent::PeriodicRebroadcast = intent {
				return do_rebroadcast
			}

			let peer = match inner.peers.peer(who) {
				None => return false,
				Some(x) => x,
			};

			// if the topic is not something we're keeping at the moment,
			// do not send.
			let (maybe_round, set_id) = match inner.live_topics.topic_info(&topic) {
				None => return false,
				Some(x) => x,
			};

			if let MessageIntent::Broadcast = intent {
				if maybe_round.is_some() {
					if !inner.round_message_allowed(who) {
						// early return if the vote message isn't allowed at this stage.
						return false
					}
				} else {
					if !inner.global_message_allowed(who) {
						// early return if the global message isn't allowed at this stage.
						return false
					}
				}
			}

			// if the topic is not something the peer accepts, discard.
			if let Some(round) = maybe_round {
				return peer.round.consider_vote(round, set_id) == Consider::Accept
			}

			// global message.
			let local_round = match inner.local_round {
				Some(ref v) => v,
				None => return false, // cannot evaluate until we have a local round.
			};

			match GossipMessage::<Block>::decode(&mut data) {
				Err(_) => false,
				Ok(GossipMessage::Commit(full)) => {
					// we only broadcast commit messages if they're for the same
					// set the peer is in and if the commit is better than the
					// last received by peer, additionally we make sure to only
					// broadcast our best commit.
					peer.round.consider_global(set_id, full.message.target_number) ==
						Consider::Accept && Some(&full.message.target_number) ==
						local_round.last_commit_height()
				},
				Ok(GossipMessage::Global(msg)) => {
					log::debug!(target:"afp", "global message in not_allowed: {:?}", msg);
					// TODO: consider filtering it further.
					true
				},
				Ok(GossipMessage::Neighbor(_)) => false,
				// Ok(GossipMessage::CatchUpRequest(_)) => false,
				// Ok(GossipMessage::CatchUp(msg)) => {
				// 	log::trace!(target:"afp", "catch-up message in not_allowed: {:?}", msg);
				// 	false
				// },
				Ok(GossipMessage::Vote(_)) => false, // should not be the case.
			}
		})
	}

	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		let inner = self.inner.read();
		Box::new(move |topic, mut data| {
			// if the topic is not one of the ones that we are keeping at the moment,
			// it is expired.
			match inner.live_topics.topic_info(&topic) {
				None => return true,
				// round messages don't require further checking.
				Some((Some(_), _)) => return false,
				Some((None, _)) => {},
			};

			let local_round = match inner.local_round {
				Some(ref v) => v,
				None => return true, // no local round means we can't evaluate or hold any topic.
			};

			// global messages -- only keep the best commit.
			match GossipMessage::<Block>::decode(&mut data) {
				Err(_) => true,
				Ok(GossipMessage::Commit(full)) => match local_round.last_commit {
					Some((number, round, set_id)) =>
					// we expire any commit message that doesn't target the same block
					// as our best commit or isn't from the same round and set id
						!(full.message.target_number == number &&
							full.round == round && full.set_id == set_id),
					None => true,
				},
				Ok(_) => true,
			}
		})
	}
}

/// Report specifying a reputation change for a given peer.
pub(super) struct PeerReport {
	pub who: PeerId,
	pub cost_benefit: ReputationChange,
}
