use std::{
	cmp::max,
	collections::VecDeque,
	env,
	pin::Pin,
	sync::{Arc, Mutex},
	task::{Context, Poll},
};

use async_recursion::async_recursion;
use futures::{channel::mpsc::Receiver as Recv, Future, StreamExt};

use log::{debug, error, info, trace};
use parity_scale_codec::{Decode, Encode};
use tokio::sync::mpsc::{channel, Receiver, Sender};

use sc_client_api::{Backend, CallExecutor};
use sc_network::types::ProtocolName;
use sc_network_gossip::TopicNotification;
use sp_application_crypto::AppCrypto;
use sp_blockchain::BlockStatus;
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthoritySignature, HOTSTUFF_KEY_TYPE};
use sp_core::{crypto::ByteArray, traits::CallContext};
use sp_keystore::KeystorePtr;
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Hash as HashT, Header as HeaderT, Zero},
};

use crate::{
	aggregator::Aggregator,
	client::{ClientForHotstuff, LinkHalf},
	import::{BlockInfo, PendingFinalizeBlockQueue},
	message::{ConsensusMessage, ConsensusMessage::*, Payload, Proposal, Timeout, Vote, QC, TC},
	network::{HotstuffNetworkBridge, Network as NetworkT, Syncing as SyncingT},
	primitives::{HotstuffError, HotstuffError::*, ViewNumber},
	synchronizer::{Synchronizer, Timer},
};

#[cfg(test)]
#[path = "tests/consensus_tests.rs"]
pub mod consensus_tests;

pub(crate) const EMPTY_PAYLOAD: &[u8] = b"hotstuff/empty_payload";

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
		payload: Payload<B>,
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
	sync: S,
	local_timer: Timer,
	synchronizer: Synchronizer<B, BE, C>,
	_consensus_msg_tx: Sender<ConsensusMessage<B>>,
	consensus_msg_rx: Receiver<ConsensusMessage<B>>,

	processing_block: Option<BlockInfo<B>>,
	pending_finalize_queue: Arc<Mutex<VecDeque<BlockInfo<B>>>>,

	proposal_hash_queue: Vec<B::Hash>,
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
		sync: S,
		network: HotstuffNetworkBridge<B, N, S>,
		synchronizer: Synchronizer<B, BE, C>,
		local_timer_duration: u64,
		consensus_msg_tx: Sender<ConsensusMessage<B>>,
		consensus_msg_rx: Receiver<ConsensusMessage<B>>,
		pending_finalize_queue: Arc<Mutex<VecDeque<BlockInfo<B>>>>,
	) -> Self {
		let pending_block = pending_finalize_queue.lock().ok().and_then(|q| q.front().cloned());

		Self {
			state: consensus_state,
			network,
			local_timer: Timer::new(local_timer_duration),
			_consensus_msg_tx: consensus_msg_tx,
			consensus_msg_rx,
			client,
			sync,
			synchronizer,
			processing_block: pending_block,
			pending_finalize_queue,
			proposal_hash_queue: Vec::new(),
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
							Err(e) =>  debug!(target: "Hotstuff","{:#?} handle_proposal has error {:#?}",self.state.local_authority_id(), e),
						};
						Ok(())
					},
					Vote(vote) => {
						match self.handle_vote(&vote).await{
							Ok(_) => {},
							Err(e) => debug!(target: "Hotstuff","handle_vote has error {:#?}", e),
						};
						Ok(())
					},
					Timeout(timeout) => {
						match self.handle_timeout(&timeout).await{
							Ok(_) => {},
							Err(e) => debug!(target: "Hotstuff","{:#?} handle_timeout has error {:#?}",self.state.local_authority_id(), e),
						};
						Ok(())
					},
					TC(tc) => {
						match self.handle_tc(&tc).await{
							Ok(_) => {},
							Err(e) =>  debug!(target: "Hotstuff","handle_tc has error {:#?}", e),
						}
						Ok(())
					},
					_ => Ok(()),
				}
			};
		}
	}

	pub async fn handle_local_timer(&mut self) -> Result<(), HotstuffError> {
		info!(target: "Hotstuff","$L$ handle_local_timer. self.view {}", self.state.view());

		self.local_timer.reset();
		self.state.increase_last_voted_view();

		let timeout = self.state.make_timeout()?;
		let message = ConsensusMessage::Timeout(timeout.clone());

		self.network.gossip_engine.lock().gossip_message(
			ConsensusMessage::<B>::gossip_topic(),
			message.encode(),
			true,
		);

		self.handle_timeout(&timeout).await
	}

	pub async fn handle_timeout(&mut self, timeout: &Timeout<B>) -> Result<(), HotstuffError> {
		debug!(target: "Hotstuff","~~ handle_timeout. self.view {}, timeout.view {}, timeout.author {}, timeout.qc.view {}",
			self.state.view(), timeout.view, timeout.voter, timeout.high_qc.view);

		if self.state.view() > timeout.view {
			return Ok(())
		}

		self.state.verify_timeout(timeout)?;

		self.handle_qc(&timeout.high_qc);

		// self.reset_processing_block();

		if let Some(tc) = self.state.add_timeout(timeout)? {
			if tc.view >= self.state.view() {
				debug!(target: "Hotstuff","~~ handle_timeout. get TC. self.view {}, tc.view {}, timeout.qc.view {}",
					self.state.view(), tc.view, timeout.high_qc.view);

				self.advance_view(tc.view);
				self.local_timer.reset();
			}

			debug!(target: "Hotstuff","~~ handle_timeout. after get TC. self.view {}", self.state.view());

			// This Timeout has received sufficient votes to form a TC (Timeout Certificate)
			// and is broadcasted into the network. Nodes that voted for this Timeout will consider
			// it as "expired."
			let message = ConsensusMessage::TC(tc.clone());

			self.network.gossip_engine.lock().gossip_message(
				ConsensusMessage::<B>::gossip_topic(),
				message.encode(),
				true,
			);

			if self.state.is_leader() {
				info!(target: "Hotstuff","@L@ handle_timeout. leader propose. self.view {}, TC.view {}",
					self.state.view(),
					timeout.view,
				);
				self.processing_block = None;
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

		if !proposal.payload.block_hash.eq(&Self::empty_payload_hash()){
			match self.client.status(proposal.payload.block_hash) {
				Ok(block_status) =>
					if BlockStatus::Unknown == block_status {
						info!(target:"Hotstuff", "#^# unknown proposal payload {}", proposal.payload);
						self.sync.set_sync_fork_request(
							vec![],
							proposal.payload.block_hash,
							proposal.payload.block_number,
						);
						return Ok(())
					},
				Err(e) => return Err(ClientError(e.to_string())),
			}
		}

		self.state.verify_proposal(proposal)?;

		self.handle_qc(&proposal.qc);

		if let Some(tc) = proposal.tc.as_ref() {
			if tc.view > self.state.view() {
				self.advance_view(tc.view);
				self.local_timer.reset();
			}
		}

		self.synchronizer.save_proposal(proposal)?;

		// Try get proposal ancestors. If we can't get them from local store,
		// then get them by network. So should we block here.
		// TODO
		if let Err(e) =
			self.synchronizer
				.get_proposal_ancestors(proposal)
				.and_then(|(parent, grandpa)| {
					if parent.view == grandpa.view + 1 {
						debug!(target: "Hotstuff","~~ handle_proposal. block {} can finalize", grandpa.payload);

						// TODO check weather this block has already finalize.
						if grandpa.payload.block_hash != Self::empty_payload_hash() &&
							grandpa.payload.block_hash != self.client.info().finalized_hash
						{
							info!(target: "Hotstuff","^^_^^ handle_proposal. block {} can finalize", grandpa.payload);
							self.client
								.finalize_block(grandpa.payload.block_hash, None, true)
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
			debug!(target: "Hotstuff","~~ handle proposal. make vote. vote.view {}", vote.view);

			self.proposal_hash_queue.push(proposal.payload.block_hash);

			self.processing_block = Some(BlockInfo {
				hash: Some(proposal.payload.block_hash),
				number: proposal.payload.block_number,
			});

			let next_leader_id = self.state.view_leader(self.state.view() + 1);

			// If the current authority is the leader of the next view, it directly processes the
			// vote. Otherwise, it sends the vote to the next leader.
			if self.state.local_authority_id().map_or(false, |id| id == next_leader_id) {
				self.handle_vote(&vote).await?;
			} else {
				let vote_message = ConsensusMessage::Vote(vote);

				self.network.gossip_engine.lock().gossip_message(
					ConsensusMessage::<B>::gossip_topic(),
					vote_message.encode(),
					false,
				);
			}
		}

		Ok(())
	}

	pub async fn handle_vote(&mut self, vote: &Vote<B>) -> Result<(), HotstuffError> {
		debug!(target: "Hotstuff","~~ handle_vote. self.view {}, vote.view {}, vote.author {}, vote.hash {}",
			self.state.view(),
			vote.view,
			vote.voter,
			vote.proposal_hash,
		);

		// TODO check proposal is in local. If not exist, sync from network.
		self.state.verify_vote(vote)?;

		if let Some(qc) = self.state.add_vote(vote)? {
			debug!(target: "Hotstuff","~~ handle_vote. get QC. view:{}, proposal_hash:{}, self.view {}", qc.view, qc.proposal_hash, self.state.view());
			self.handle_qc(&qc);

			debug!(target: "Hotstuff","~~ handle_vote. get QC. after handle qc, self view {}", self.state.view());
			let current_leader = self.state.view_leader(self.state.view());
			if self.state.local_authority_id().map_or(false, |id| id == current_leader) {
				if let Some(payload) = self.get_proposal_payload() {
					info!(target: "Hotstuff","~~ handle_vote. make proposal. payload {}", payload);
					debug!(target: "Hotstuff", "&-& proposal_hash_queue {:#?}", self.proposal_hash_queue);

					let mut count = 0;
					for item in self.proposal_hash_queue.iter().rev() {
						if !item.eq(&Self::empty_payload_hash()) {
							break
						}
						count += 1;
						if count == 2 && payload.block_hash.eq(&Self::empty_payload_hash()) {
							info!(target:"Hotstuff", "^^ already has 3 empty proposal, this empty not gossip");
							return Ok(())
						}
					}

					if self.proposal_hash_queue.len() > 3 {
						self.proposal_hash_queue.clear()
					}

					self.proposal_hash_queue.push(payload.block_hash);
					debug!(target: "Hotstuff", "&*& proposal_hash_queue {:#?}", self.proposal_hash_queue);

					let proposal = self.state.make_proposal(payload, None)?;
					let proposal_message = ConsensusMessage::Propose(proposal.clone());

					self.network.gossip_engine.lock().gossip_message(
						ConsensusMessage::<B>::gossip_topic(),
						proposal_message.encode(),
						false,
					);

					// Inform oneself to handle the proposal.
					// self.consensus_msg_tx
					// .send(proposal_message)
					// .await
					// .map_err(|e| Other(e.to_string()))?;
					self.handle_proposal(&proposal).await?;
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
		debug!(target: "Hotstuff","~~ handle_tc. from network, self.view {}, tc.view {}",self.state.view(), tc.view);
		self.state.verify_tc(tc)?;

		self.advance_view(tc.view);
		self.local_timer.reset();
		self.processing_block = None;

		if self.state.is_leader() {
			info!(target: "Hotstuff","¥T¥ handle_tc. leader make proposal valid tc, tc.view {}, self.view {}",tc.view, self.state.view());
			self.generate_proposal(None).await?;
		}

		Ok(())
	}

	pub async fn generate_proposal(&mut self, tc: Option<TC<B>>) -> Result<(), HotstuffError> {
		match self.get_proposal_payload() {
			Some(payload) => {
				debug!(target: "Hotstuff","~~ generate_proposal. payload :{}, self.view:{}",
					payload,
					self.state.view(),
				);

				let proposal = self.state.make_proposal(payload, tc)?;
				let proposal_message = ConsensusMessage::Propose(proposal.clone());

				self.network.gossip_engine.lock().gossip_message(
					ConsensusMessage::<B>::gossip_topic(),
					proposal_message.encode(),
					false,
				);

				// TODO Inform oneself to handle the proposal by channel?
				self.handle_proposal(&proposal).await?;
			},
			None => {
				debug!(target: "Hotstuff","~~ generate_proposal. can't get substrate block.")
			},
		}

		Ok(())
	}

	pub(crate) fn get_proposal_payload(&mut self) -> Option<Payload<B>> {
		if let Ok(queue) = self.pending_finalize_queue.lock() {
			if let Some(processing) = self.processing_block.as_ref() {
				let finalized_number = self.client.info().finalized_number;
				if processing.number > finalized_number {
					trace!(target: "Hotstuff", "~~ get_proposal_payload. return a empty payload. processing: {}, finalized:{}",
						processing.number,
						finalized_number);

					return Some(Payload {
						block_hash: Self::empty_payload_hash(),
						block_number: processing.number,
					})
				}

				if let Some(find) = queue.iter().find(|elem| elem.number > finalized_number) {
					trace!(target: "Hotstuff", "~~ get_proposal_payload. get from queue. processing:{}, finalized:{}, find {}",
						processing.number,
						finalized_number,
						find.number,
					);

					self.processing_block = Some(find.clone());
					return Some(Payload {
						block_hash: find.hash.unwrap_or(Self::empty_payload_hash()),
						block_number: find.number,
					})
				} else {
					trace!(target: "Hotstuff", "~~ get_proposal_payload. get from queue failed: processing{}, finalized:{}",
						processing.number,
						finalized_number,
					);

					return Some(Payload {
						block_hash: Self::empty_payload_hash(),
						block_number: processing.number,
					})
				}
			} else {
				match queue.front().cloned() {
					Some(info) => {
						info!(target: "Hotstuff", "Q-Q get_proposal_payload. processing is None,get from queue {}", info.number);

						self.processing_block = Some(info.clone());
						return Some(Payload::<B> {
							block_hash: info.hash.unwrap_or(Self::empty_payload_hash()),
							block_number: info.number,
						})
					},
					None => return None,
				}
			}
		}

		None
	}

	fn advance_view(&mut self, view: ViewNumber) {
		self.state.advance_view_from_target(view);
		self.network.set_view(self.state.view());
	}

	pub(crate) fn empty_payload_hash() -> B::Hash {
		<<B::Header as HeaderT>::Hashing as HashT>::hash(EMPTY_PAYLOAD)
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
	pending_queue: PendingFinalizeBlockQueue<B>,
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

		match Future::poll(Pin::new(&mut self.pending_queue), cx) {
			Poll::Ready(_) => {},
			Poll::Pending => {},
		};

		match Future::poll(Pin::new(&mut self.network), cx) {
			Poll::Ready(_) => {},
			Poll::Pending => {},
		};

		Poll::Pending
	}
}

impl<B, N, S> ConsensusNetwork<B, N, S>
where
	B: BlockT,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	fn new(
		network: HotstuffNetworkBridge<B, N, S>,
		consensus_msg_tx: Sender<ConsensusMessage<B>>,
		pending_queue: PendingFinalizeBlockQueue<B>,
	) -> Self {
		let message_recv = network
			.gossip_engine
			.clone()
			.lock()
			.messages_for(ConsensusMessage::<B>::gossip_topic());

		Self { network, consensus_msg_tx, message_recv, pending_queue }
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
	C::Api: sp_consensus_hotstuff::HotstuffApi<B, AuthorityId>,
{
	let LinkHalf { client, .. } = link;
	let authorities = get_genesis_authorities_from_client::<B, BE, C>(client.clone());

	let network = HotstuffNetworkBridge::new(network.clone(), sync.clone(), hotstuff_protocol_name);
	let synchronizer = Synchronizer::<B, BE, C>::new(client.clone());
	let consensus_state = ConsensusState::<B>::new(keystore, authorities);

	let (consensus_msg_tx, consensus_msg_rx) = channel::<ConsensusMessage<B>>(1000);

	let queue = PendingFinalizeBlockQueue::<B>::new(client.clone()).expect("error");

	let mut local_timer_duration = 3000;
	if let Ok(value) = env::var("HOTSTUFF_DURATION") {
		if let Ok(duration) = value.parse::<u64>() {
			local_timer_duration = duration;
		}
	}

	let consensus_worker = ConsensusWorker::<B, BE, C, N, S>::new(
		consensus_state,
		client,
		sync,
		network.clone(),
		synchronizer,
		local_timer_duration,
		consensus_msg_tx.clone(),
		consensus_msg_rx,
		queue.queue(),
	);

	let consensus_network = ConsensusNetwork::<B, N, S>::new(network, consensus_msg_tx, queue);

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
