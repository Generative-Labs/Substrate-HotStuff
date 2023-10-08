use std::{
	cmp::max,
	collections::HashSet,
	pin::Pin,
	sync::Arc,
	task::{Context, Poll},
};

use async_recursion::async_recursion;
use futures::{channel::mpsc::Receiver as Recv, Future, StreamExt};

use log::{error, info};
use parity_scale_codec::{Decode, Encode};
use tokio::sync::mpsc::{channel, Receiver, Sender};

use sc_client_api::{Backend, CallExecutor};
use sc_network::types::ProtocolName;
use sc_network_gossip::TopicNotification;
use sc_utils::mpsc::TracingUnboundedReceiver;
use sp_application_crypto::AppCrypto;
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthoritySignature, HOTSTUFF_KEY_TYPE};
use sp_core::{crypto::ByteArray, traits::CallContext};
use sp_keystore::KeystorePtr;
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Hash as HashT, Header as HeaderT, NumberFor, Zero},
};

use crate::{
	aggregator::Aggregator,
	client::{ClientForHotstuff, LinkHalf},
	message::{ConsensusMessage, ConsensusMessage::*, Proposal, Timeout, Vote, QC, TC},
	network::{HotstuffNetworkBridge, Network as NetworkT, Syncing as SyncingT},
	primitives::{HotstuffError, HotstuffError::*, ViewNumber},
	synchronizer::{Synchronizer, Timer},
};

#[cfg(test)]
#[path = "tests/consensus_tests.rs"]
pub mod consensus_tests;

// the core of hotstuff
pub struct ConsensusState<B: BlockT> {
	keystore: KeystorePtr,
	authorities: AuthorityList,
	view: ViewNumber,
	last_voted_view: ViewNumber,
	// last_committed_round: ViewNumber,
	high_qc: QC<B>,
	aggregator: Aggregator<B>,
}

impl<B: BlockT> ConsensusState<B> {
	pub fn new(keystore: KeystorePtr, authorities: AuthorityList) -> Self {
		Self {
			keystore,
			authorities,
			view: 0,
			last_voted_view: 0,
			high_qc: Default::default(),
			aggregator: Aggregator::<B>::new(),
		}
	}

	// find local authority id. If the result is None, local node is not authority.
	// TODO no loop. just init in construct function ?
	pub fn local_authority_id(&self) -> Option<AuthorityId> {
		self.authorities
			.iter()
			.find(|(p, _)| self.keystore.has_keys(&[(p.to_raw_vec(), HOTSTUFF_KEY_TYPE)]))
			.map(|(p, _)| p.clone())
	}

	pub fn increase_last_voted_view(&mut self) {
		self.last_voted_view = max(self.last_voted_view, self.view)
	}

	pub fn make_timeout(&self) -> Result<Timeout<B>, HotstuffError> {
		let authority_id = self.local_authority_id().ok_or(NotAuthority)?;

		let mut tc: Timeout<B> = Timeout {
			high_qc: self.high_qc.clone(),
			view: self.view,
			voter: authority_id.clone(),
			signature: None,
		};

		tc.signature = self
			.keystore
			.sign_with(
				AuthorityId::ID,
				AuthorityId::CRYPTO_ID,
				authority_id.as_ref(),
				tc.digest().as_ref(),
			)
			.map_err(|e| Other(e.to_string()))?
			.and_then(|data| AuthoritySignature::try_from(data).ok());

		Ok(tc)
	}

	pub fn make_proposal(
		&self,
		payload: B::Hash,
		tc: Option<TC<B>>,
	) -> Result<Proposal<B>, HotstuffError> {
		let author_id = self.local_authority_id().ok_or(NotAuthority)?;
		let mut block = Proposal::<B>::new(
			self.high_qc.clone(),
			tc,
			payload,
			self.view,
			author_id.clone(),
			None,
		);

		block.signature = self
			.keystore
			.sign_with(
				AuthorityId::ID,
				AuthorityId::CRYPTO_ID,
				author_id.as_slice(),
				block.digest().as_ref(),
			)
			.map_err(|e| Other(e.to_string()))?
			.and_then(|data| AuthoritySignature::try_from(data).ok());

		Ok(block)
	}

	pub fn make_vote(&mut self, proposal: &Proposal<B>) -> Option<Vote<B>> {
		let author_id = self.local_authority_id()?;

		if proposal.view <= self.last_voted_view {
			return None
		}

		// TODO how process TC of proposal.
		self.last_voted_view = max(self.last_voted_view, proposal.view);

		let mut vote = Vote::<B>::new(proposal.digest(), proposal.view, author_id.clone());

		vote.signature = self
			.keystore
			.sign_with(
				AuthorityId::ID,
				AuthorityId::CRYPTO_ID,
				author_id.as_slice(),
				vote.digest().as_ref(),
			)
			.ok()?
			.and_then(|data| AuthoritySignature::try_from(data).ok());

		Some(vote)
	}

	pub fn view(&self) -> ViewNumber {
		self.view
	}

	pub fn verify_timeout(&self, timeout: &Timeout<B>) -> Result<(), HotstuffError> {
		timeout.verify(&self.authorities)
	}

	pub fn verify_proposal(&self, proposal: &Proposal<B>) -> Result<(), HotstuffError> {
		if !proposal.author.eq(&self.view_leader(proposal.view)) {
			return Err(WrongProposer)
		}

		// TODO how process authority changed.
		proposal.verify(&self.authorities)
	}

	pub fn verify_vote(&self, vote: &Vote<B>) -> Result<(), HotstuffError> {
		if vote.view < self.view {
			return Err(ExpiredVote)
		}

		vote.verify(&self.authorities)
	}

	pub fn verify_tc(&self, tc: &TC<B>) -> Result<(), HotstuffError> {
		if tc.view < self.view {
			return Err(InvalidTC)
		}

		tc.verify(&self.authorities)
	}

	// add a verified timeout then try return a TC.
	pub fn add_timeout(&mut self, timeout: &Timeout<B>) -> Result<Option<TC<B>>, HotstuffError> {
		self.aggregator.add_timeout(timeout, &self.authorities)
	}

	// add a verified vote and try return a QC.
	pub fn add_vote(&mut self, vote: &Vote<B>) -> Result<Option<QC<B>>, HotstuffError> {
		self.aggregator.add_vote(vote.clone(), &self.authorities)
	}

	pub fn update_high_qc(&mut self, qc: &QC<B>) {
		if qc.view > self.high_qc.view {
			self.high_qc = qc.clone()
		}
	}

	pub fn advance_view_from_target(&mut self, view: ViewNumber) {
		if self.view >= view {
			self.view = view + 1;
		}
	}

	pub fn view_leader(&self, view: ViewNumber) -> AuthorityId {
		let leader_index = view % self.authorities.len() as ViewNumber;
		self.authorities[leader_index as usize].0.clone()
	}

	// hotstuff consensus leader, not substrate block author.
	pub fn is_leader(&self) -> bool {
		let leader_index = self.view % self.authorities.len() as ViewNumber;
		let leader_id = &self.authorities[leader_index as usize].0;

		if let Some(id) = self.local_authority_id() {
			return id.eq(leader_id)
		}

		false
	}
}

pub struct ConsensusWorker<
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
> {
	state: ConsensusState<B>,

	network: HotstuffNetworkBridge<B, N, S>,
	client: Arc<C>,
	local_timer: Timer,
	synchronizer: Synchronizer<B, BE, C>,
	import_block_rx: TracingUnboundedReceiver<(B::Hash, NumberFor<B>)>,
	_consensus_msg_tx: Sender<ConsensusMessage<B>>,
	consensus_msg_rx: Receiver<ConsensusMessage<B>>,

	// Sometimes, when we have already voted for a proposal from a peer,
	// but we haven't consumed the block hash sent to us by BlockImport,
	// this can potentially lead to different proposals having the same payload.
	// Therefore, we need to keep a record of the processed block hashes.
	processed_block_set: HashSet<B::Hash>,
}

impl<B, BE, C, N, S> ConsensusWorker<B, BE, C, N, S>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	// TODO now the construct function is developing.
	#![allow(clippy::too_many_arguments)]
	pub fn new(
		consensus_state: ConsensusState<B>,
		client: Arc<C>,
		network: HotstuffNetworkBridge<B, N, S>,
		synchronizer: Synchronizer<B, BE, C>,
		local_timer_duration: u64,
		consensus_msg_tx: Sender<ConsensusMessage<B>>,
		consensus_msg_rx: Receiver<ConsensusMessage<B>>,
		import_block_rx: TracingUnboundedReceiver<(B::Hash, NumberFor<B>)>,
	) -> Self {
		// TODO channel size?
		Self {
			state: consensus_state,
			network,
			local_timer: Timer::new(local_timer_duration),
			_consensus_msg_tx: consensus_msg_tx,
			consensus_msg_rx,
			client,
			synchronizer,
			import_block_rx,
			processed_block_set: HashSet::new(),
		}
	}

	pub async fn run(mut self) {
		loop {
			let _ = tokio::select! {
				_ = &mut self.local_timer => self.handle_local_timer().await,
				Some(message) = self.consensus_msg_rx.recv()=> match message {
					Propose(proposal) => {
						match self.handle_proposal(&proposal).await{
							Ok(_) => {},
							Err(e) =>  info!(target: "Hotstuff","{:#?} handle_proposal has error {:#?}",self.state.local_authority_id(), e),
						};
						Ok(())
					},
					Vote(vote) => {
						match self.handle_vote(&vote).await{
							Ok(_) => {},
							Err(e) => info!(target: "Hotstuff","handle_vote has error {:#?}", e),
						};
						Ok(())
					},
					Timeout(timeout) => {
						match self.handle_timeout(&timeout).await{
							Ok(_) => {},
							Err(e) => info!(target: "Hotstuff","{:#?} handle_timeout has error {:#?}",self.state.local_authority_id(), e),
						};
						Ok(())
					},
					TC(tc) => {
						match self.handle_tc(&tc).await{
							Ok(_) => {},
							Err(e) =>  info!(target: "Hotstuff","handle_tc has error {:#?}", e),
						}
						Ok(())
					},
					_ => Ok(()),
				}
			};
		}
	}

	pub async fn handle_local_timer(&mut self) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","~~ handle_local_timer. self.view {}", self.state.view());

		self.local_timer.reset();
		self.state.increase_last_voted_view();

		let timeout = self.state.make_timeout()?;
		let message = ConsensusMessage::Timeout(timeout.clone());

		self.network
			.gossip_engine
			.lock()
			.register_gossip_message(ConsensusMessage::<B>::gossip_topic(), message.encode());

		self.handle_timeout(&timeout).await
	}

	pub async fn handle_timeout(&mut self, timeout: &Timeout<B>) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","~~ handle_timeout. self.view {}, timeout.view {}, timeout.author {}, timeout.qc.view {}",
			self.state.view(), timeout.view, timeout.voter, timeout.high_qc.view);

		if self.state.view() > timeout.view {
			return Ok(())
		}

		self.state.verify_timeout(timeout)?;

		self.handle_qc(&timeout.high_qc);

		if let Some(tc) = self.state.add_timeout(timeout)? {
			if tc.view >= self.state.view() {
				info!(target: "Hotstuff","~~ handle_timeout. get TC. self.view {}, tc.view {}, timeout.qc.view {}",
					self.state.view(), tc.view, timeout.high_qc.view);

				self.advance_view(tc.view);
				self.local_timer.reset();
			}

			info!(target: "Hotstuff","~~ handle_timeout. after get TC. self.view {}", self.state.view());

			// This Timeout has received sufficient votes to form a TC (Timeout Certificate)
			// and is broadcasted into the network. Nodes that voted for this Timeout will consider
			// it as "expired."
			let message = ConsensusMessage::TC(tc.clone());
			self.network
				.gossip_engine
				.lock()
				.register_gossip_message(ConsensusMessage::<B>::gossip_topic(), message.encode());

			if self.state.is_leader() {
				info!(target: "Hotstuff","~~ handle_timeout. leader make after valid TC. self.view {}, TC.view {}", self.state.view(), timeout.view);

				self.generate_proposal(Some(tc)).await?;
			}
		}

		Ok(())
	}

	#[async_recursion]
	pub async fn handle_proposal(&mut self, proposal: &Proposal<B>) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","~~ handle_proposal. self.view {}, proposal[ view:{},  payload:{}, author {}, digest {}]",
			self.state.view(),
			proposal.view,
			proposal.payload,
			proposal.author,
			proposal.digest(),
		);

		self.state.verify_proposal(proposal)?;

		self.handle_qc(&proposal.qc);

		if let Some(tc) = proposal.tc.as_ref() {
			if tc.view > self.state.view() {
				self.advance_view(tc.view);
				self.local_timer.reset();
			}
		}

		self.synchronizer.save_proposal(proposal)?;
		self.processed_block_set.insert(proposal.payload);

		// Try get proposal ancestors. If we can't get them from local store,
		// then get them by network. So should we block here.
		// TODO
		if let Err(e) =
			self.synchronizer
				.get_proposal_ancestors(proposal)
				.and_then(|(parent, grandpa)| {
					if parent.view == grandpa.view + 1 {
						info!(target: "Hotstuff","~~ handle_proposal. block {} can finalize", grandpa.payload);

						// TODO check weather this block has already finalize.
						if grandpa.payload != Self::empty_payload() {
							self.client
								.finalize_block(grandpa.payload, None, true)
								.map_err(|e| FinalizeBlock(e.to_string()))?;
						}
					}
					Ok(())
				}) {
			info!(target: "Hotstuff", "~~ handle_proposal. has error when finalize block {:#?}", e);
		}

		if proposal.view != self.state.view() {
			return Ok(())
		}

		if let Some(vote) = self.state.make_vote(proposal) {
			info!(target: "Hotstuff","~~ handle proposal. make vote. vote.view {}", vote.view);

			let next_leader_id = self.state.view_leader(self.state.view() + 1);

			// If the current authority is the leader of the next view, it directly processes the
			// vote. Otherwise, it sends the vote to the next leader.
			if self.state.local_authority_id().map_or(false, |id| id == next_leader_id) {
				self.handle_vote(&vote).await?;
			} else {
				let vote_message = ConsensusMessage::Vote(vote);

				self.network.gossip_engine.lock().register_gossip_message(
					ConsensusMessage::<B>::gossip_topic(),
					vote_message.encode(),
				);
			}
		}

		Ok(())
	}

	pub async fn handle_vote(&mut self, vote: &Vote<B>) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","~~ handle_vote. self.view {}, vote.view {}, vote.author {}, vote.hash {}",
			self.state.view(),
			vote.view,
			vote.voter,
			vote.proposal_hash,
		);

		self.state.verify_vote(vote)?;

		if let Some(qc) = self.state.add_vote(vote)? {
			info!(target: "Hotstuff","~~ handle_vote. get QC. view:{}, proposal_hash:{}, self.view {}", qc.view, qc.proposal_hash, self.state.view());
			self.handle_qc(&qc);

			info!(target: "Hotstuff","~~ handle_vote. get QC. after handle qc, self view {}", self.state.view());
			let current_leader = self.state.view_leader(self.state.view());
			if self.state.local_authority_id().map_or(false, |id| id == current_leader) {
				if let Some(hash) = self.get_imported_block() {
					info!(target: "Hotstuff","~~ handle_vote. make proposal, substrate block hash {}", hash);

					let block = self.state.make_proposal(hash, None)?;
					let proposal_message = ConsensusMessage::Propose(block.clone());

					self.network.gossip_engine.lock().register_gossip_message(
						ConsensusMessage::<B>::gossip_topic(),
						proposal_message.encode(),
					);

					// Inform oneself to handle the proposal.
					// self.consensus_msg_tx
					// .send(proposal_message)
					// .await
					// .map_err(|e| Other(e.to_string()))?;
					self.handle_proposal(&block).await?;
				}
			}
		}

		Ok(())
	}

	pub fn handle_qc(&mut self, qc: &QC<B>) {
		if qc.view >= self.state.view() {
			self.advance_view(qc.view);
			self.state.update_high_qc(qc);
			self.local_timer.reset();
		}
	}

	pub async fn handle_tc(&mut self, tc: &TC<B>) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","~~ handle_tc. from network, self.view {}, tc.view {}",self.state.view(), tc.view);
		self.state.verify_tc(tc)?;

		self.advance_view(tc.view);
		self.local_timer.reset();

		if self.state.is_leader() {
			info!(target: "Hotstuff","~~ handle_tc. leader make proposal valid tc, tc.view {}, self.view {}",tc.view, self.state.view());
			self.generate_proposal(None).await?;
		}

		Ok(())
	}

	pub async fn generate_proposal(&mut self, tc: Option<TC<B>>) -> Result<(), HotstuffError> {
		match self.get_imported_block() {
			Some(hash) => {
				info!(target: "Hotstuff","~~ generate_proposal. substrate block_hash:{}, self.view:{}",
					hash,
					self.state.view(),
				);

				let proposal = self.state.make_proposal(hash, tc)?;
				let proposal_message = ConsensusMessage::Propose(proposal.clone());

				self.network.gossip_engine.lock().register_gossip_message(
					ConsensusMessage::<B>::gossip_topic(),
					proposal_message.encode(),
				);

				// TODO Inform oneself to handle the proposal by channel?
				self.handle_proposal(&proposal).await?;
			},
			None => {
				info!(target: "Hotstuff","~~ generate_proposal. can't get substrate block.")
			},
		}

		Ok(())
	}

	fn get_imported_block(&mut self) -> Option<B::Hash> {
		while let Ok(hash) = self.import_block_rx.try_recv() {
			if self.processed_block_set.contains(&hash.0) {
				let _ = self.processed_block_set.remove(&hash.0);
				continue
			}
			return Some(hash.0)
		}

		Some(Self::empty_payload())
	}

	fn advance_view(&mut self, view: ViewNumber){
		self.state.advance_view_from_target(view);
		self.network.set_view(self.state.view());
	}

	fn empty_payload() -> B::Hash {
		<<B::Header as HeaderT>::Hashing as HashT>::hash(b"hotstuff/empty_payload")
	}
}

impl<B, N, S> Future for ConsensusNetwork<B, N, S>
where
	B: BlockT,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		loop {
			match StreamExt::poll_next_unpin(&mut self.message_recv, cx) {
				Poll::Ready(None) => break,
				Poll::Ready(Some(notification)) => {
					if let Err(e) = self.incoming_message_handler(notification) {
						error!("process incoming message error: {:#?}", e)
					}
				},
				Poll::Pending => break,
			};
		}

		match Future::poll(Pin::new(&mut self.network), cx) {
			Poll::Ready(_) => {},
			Poll::Pending => {},
		};

		Poll::Pending
	}
}

pub struct ConsensusNetwork<
	B: BlockT,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
> {
	network: HotstuffNetworkBridge<B, N, S>,
	message_recv: Recv<TopicNotification>,
	consensus_msg_tx: Sender<ConsensusMessage<B>>,
}

impl<B, N, S> ConsensusNetwork<B, N, S>
where
	B: BlockT,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	pub fn new(
		network: HotstuffNetworkBridge<B, N, S>,
		consensus_msg_tx: Sender<ConsensusMessage<B>>,
	) -> Self {
		let message_recv = network
			.gossip_engine
			.clone()
			.lock()
			.messages_for(ConsensusMessage::<B>::gossip_topic());

		Self { network, consensus_msg_tx, message_recv }
	}

	pub fn incoming_message_handler(
		&mut self,
		notification: TopicNotification,
	) -> Result<(), HotstuffError> {
		let message: ConsensusMessage<B> =
			Decode::decode(&mut &notification.message[..]).map_err(|e| Other(e.to_string()))?;

		self.consensus_msg_tx.try_send(message).map_err(|e| Other(e.to_string()))
	}
}

impl<B, BE, C, N, S> Unpin for ConsensusWorker<B, BE, C, N, S>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
}

pub fn start_hotstuff<B, BE, C, N, S, SC>(
	network: N,
	link: LinkHalf<B, C, SC>,
	sync: S,
	hotstuff_protocol_name: ProtocolName,
	keystore: KeystorePtr,
) -> sp_blockchain::Result<(impl Future<Output = ()> + Send, impl Future<Output = ()> + Send)>
where
	B: BlockT,
	BE: Backend<B> + 'static,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
	C: ClientForHotstuff<B, BE> + 'static,
	C::Api: sp_consensus_grandpa::GrandpaApi<B>,
{
	let LinkHalf { client, import_block_rx, .. } = link;
	let authorities = get_genesis_authorities_from_client::<B, BE, C>(client.clone());

	let network = HotstuffNetworkBridge::new(network.clone(), sync.clone(), hotstuff_protocol_name);
	let synchronizer = Synchronizer::<B, BE, C>::new(client.clone());
	let consensus_state = ConsensusState::<B>::new(keystore, authorities);

	let (consensus_msg_tx, consensus_msg_rx) = channel::<ConsensusMessage<B>>(1000);

	let consensus_worker = ConsensusWorker::<B, BE, C, N, S>::new(
		consensus_state,
		client,
		network.clone(),
		synchronizer,
		3000,
		consensus_msg_tx.clone(),
		consensus_msg_rx,
		import_block_rx,
	);

	let consensus_network = ConsensusNetwork::<B, N, S>::new(network, consensus_msg_tx);

	Ok((async { consensus_worker.run().await }, consensus_network))
}

// TODO just for dev!
pub fn get_genesis_authorities_from_client<
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
>(
	client: Arc<C>,
) -> AuthorityList {
	let genesis_block_hash = client
		.expect_block_hash_from_id(&BlockId::Number(Zero::zero()))
		.expect("get genesis block hash from client failed");

	let authorities_data = client
		.executor()
		.call(genesis_block_hash, "HotstuffApi_authorities", &[], CallContext::Offchain)
		.expect("call runtime failed");

	let authorities: Vec<AuthorityId> = Decode::decode(&mut &authorities_data[..]).expect("");

	authorities.iter().map(|id| (id.clone(), 0)).collect::<AuthorityList>()
}

#[cfg(test)]
pub fn start_hotstuff_with_authority<B, BE, C, N, S, SC>(
	network: N,
	link: LinkHalf<B, C, SC>,
	sync: S,
	hotstuff_protocol_name: ProtocolName,
	keystore: KeystorePtr,
	authorities: AuthorityList,
) -> sp_blockchain::Result<(impl Future<Output = ()> + Send, impl Future<Output = ()> + Send)>
where
	B: BlockT,
	BE: Backend<B> + 'static,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
	C: ClientForHotstuff<B, BE> + 'static,
	C::Api: sp_consensus_grandpa::GrandpaApi<B>,
{
	let LinkHalf { client, import_block_rx, .. } = link;

	let network = HotstuffNetworkBridge::new(network.clone(), sync.clone(), hotstuff_protocol_name);
	let synchronizer = Synchronizer::<B, BE, C>::new(client.clone());
	let consensus_state = ConsensusState::<B>::new(keystore, authorities);

	let (consensus_msg_tx, consensus_msg_rx) = channel::<ConsensusMessage<B>>(1000);

	let consensus_worker = ConsensusWorker::<B, BE, C, N, S>::new(
		consensus_state,
		client,
		network.clone(),
		synchronizer,
		2000,
		consensus_msg_tx.clone(),
		consensus_msg_rx,
		import_block_rx,
	);

	let consensus_network = ConsensusNetwork::<B, N, S>::new(network, consensus_msg_tx);

	Ok((async { consensus_worker.run().await }, consensus_network))
}
