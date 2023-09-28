use std::{
	cmp::max,
	future::Future,
	ops::Add,
	pin::Pin,
	task::{Context, Poll},
};

use futures::StreamExt;
use log::{error, info};
use parity_scale_codec::{Decode, Encode};
use sc_network_gossip::TopicNotification;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use sc_client_api::Backend;
use sc_network::types::ProtocolName;
use sp_application_crypto::AppCrypto;
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthoritySignature, HOTSTUFF_KEY_TYPE};
use sp_core::crypto::ByteArray;
use sp_keystore::KeystorePtr;
use sp_runtime::traits::{Block as BlockT, One};

use crate::{
	aggregator::Aggregator,
	client::{ClientForHotstuff, LinkHalf},
	message::{ConsensusMessage, ConsensusMessage::*, Proposal, Timeout, Vote, QC, TC},
	network_bridge::{HotstuffNetworkBridge, Network as NetworkT, Syncing as SyncingT},
	primitives::{HotstuffError, HotstuffError::*, ViewNumber},
	synchronizer::{Synchronizer, Timer},
};

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
				authority_id.as_slice(),
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

		let mut vote = Vote::<B> {
			hash: proposal.digest(),
			view: proposal.view,
			voter: author_id.clone(),
			signature: None,
		};

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
		if proposal.author.eq(&self.view_leader(proposal.view)) {
			return Err(WrongProposal)
		}

		// TODO how process authority changed.
		proposal.verify(&self.authorities)
	}

	pub fn verify_vote(&self, vote: &Vote<B>) -> Result<(), HotstuffError> {
		if vote.view < self.view {
			return Err(InvalidVote)
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
		self.aggregator.add_timeout(&timeout, &self.authorities)
	}

	// add a verified vote and try return a QC.
	pub fn add_vote(&mut self, vote: &Vote<B>) -> Result<Option<QC<B>>, HotstuffError> {
		self.aggregator.add_vote(vote.clone(), &self.authorities)
	}

	pub fn update_high_qc(&mut self, qc: &QC<B>) {
		if self.high_qc.view < qc.view {
			self.high_qc = qc.clone()
		}
	}

	pub fn advance_view(&mut self) -> bool {
		self.view += 1;

		return true
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
			return id.eq(&leader_id)
		}
		return false
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
	client: C,
	local_timer: Timer,
	synchronizer: Synchronizer<B, BE, C>,

	consensus_msg_tx: Sender<ConsensusMessage<B>>,
	consensus_msg_rx: Receiver<ConsensusMessage<B>>,
}

impl<B, BE, C, N, S> ConsensusWorker<B, BE, C, N, S>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	pub fn new(
		consensus_state: ConsensusState<B>,
		client: C,
		network: HotstuffNetworkBridge<B, N, S>,
		synchronizer: Synchronizer<B, BE, C>,
		local_timer_duration: u64,
	) -> Self {
		// TODO channel size?
		let (consensus_msg_tx, consensus_msg_rx) = channel::<ConsensusMessage<B>>(1000);
		Self {
			state: consensus_state,
			network,
			local_timer: Timer::new(local_timer_duration),
			consensus_msg_tx,
			consensus_msg_rx,
			client,
			synchronizer,
		}
	}

	pub async fn run(&mut self) {
		loop {
			let result = tokio::select! {
				_ = &mut self.local_timer => self.handle_local_timer().await,
				Some(message) = self.consensus_msg_rx.recv()=> match message {
					Propose(proposal) => self.handle_proposal(&proposal).await,
					Vote(vote) => self.handle_vote(&vote).await,
					Timeout(timeout) => self.handle_timeout(&timeout).await,
					TC(tc) => self.handle_tc(&tc).await,
					_ => Ok(()),
				}
			};

			if let Err(e) = result {
				error!("ConsensusWorker has error: {:#?}", e)
			}
		}
	}

	pub async fn handle_local_timer(&mut self) -> Result<(), HotstuffError> {
		info!("local timeout, id: {}", self.network.local_peer_id());

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
		if self.state.view() > timeout.view {
			return Ok(())
		}

		self.state.verify_timeout(timeout)?;
		self.handle_qc(&timeout.high_qc);

		if let Some(tc) = self.state.add_timeout(timeout)? {
			if tc.view > self.state.view() {
				self.state.advance_view();
				self.local_timer.reset();
			}

			let message = ConsensusMessage::TC(tc.clone());

			self.network
				.gossip_engine
				.lock()
				.register_gossip_message(ConsensusMessage::<B>::gossip_topic(), message.encode());

			if self.state.is_leader() {
				self.generate_proposal(Some(tc)).await?;
			}
		}

		Ok(())
	}

	pub async fn handle_proposal(&mut self, proposal: &Proposal<B>) -> Result<(), HotstuffError> {
		self.state.verify_proposal(proposal)?;

		self.handle_qc(&proposal.qc);
		proposal.tc.as_ref().map(|tc| {
			if tc.view > self.state.view() {
				self.state.advance_view();
				self.local_timer.reset();
			}
		});

		// Try get proposal ancestors. If we can't get them from local store,
		// then get them by network. So should we block here.
		let (b0, b1) = self.synchronizer.get_proposal_ancestors(proposal)?;
		self.synchronizer.save_proposal(proposal)?;

		if b0.view + 1 == b1.view {
			info!("block {} can finalize", b0.payload);
			self.client
				.finalize_block(b0.payload, None, true)
				.map_err(|e| Other(e.to_string()))?;
		}

		if proposal.view != self.state.view() {
			return Ok(())
		}

		if let Some(vote) = self.state.make_vote(&proposal) {
			let next_leader = self.state.view_leader(self.state.view() + 1);
			if self.state.local_authority_id().map_or(false, |id| id == next_leader) {
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
		self.state.verify_vote(vote)?;

		if let Some(qc) = self.state.add_vote(vote)? {
			self.handle_qc(&qc);

			let next_leader = self.state.view_leader(self.state.view() + 1);
			if self.state.local_authority_id().map_or(false, |id| id == next_leader) {
				if let Some(hash) = self.get_finalize_block_hash() {
					let block = self.state.make_proposal(hash, None)?;
					let proposal_message = ConsensusMessage::Propose(block);

					self.network.gossip_engine.lock().register_gossip_message(
						ConsensusMessage::<B>::gossip_topic(),
						proposal_message.encode(),
					);

					// Inform oneself to handle the proposal.
					self.consensus_msg_tx
						.send(proposal_message)
						.await
						.map_err(|e| Other(e.to_string()))?;
				}
			}
		}

		Ok(())
	}

	pub fn handle_qc(&mut self, qc: &QC<B>) {
		if qc.view > self.state.view() {
			self.state.advance_view();
			self.state.update_high_qc(qc);
			self.local_timer.reset();
		}
	}

	pub async fn handle_tc(&mut self, tc: &TC<B>) -> Result<(), HotstuffError> {
		self.state.verify_tc(tc)?;

		self.state.advance_view();
		self.local_timer.reset();

		if self.state.is_leader() {
			self.generate_proposal(None).await?;
		}

		Ok(())
	}

	pub async fn generate_proposal(&mut self, tc: Option<TC<B>>) -> Result<(), HotstuffError> {
		if let Some(hash) = self.get_finalize_block_hash() {
			let block = self.state.make_proposal(hash, tc)?;
			let proposal_message = ConsensusMessage::Propose(block);

			self.network.gossip_engine.lock().register_gossip_message(
				ConsensusMessage::<B>::gossip_topic(),
				proposal_message.encode(),
			);

			// Inform oneself to handle the proposal.
			self.consensus_msg_tx
				.send(proposal_message)
				.await
				.map_err(|e| Other(e.to_string()))?;
		}

		Ok(())
	}

	// get the son of the last finalized block.
	// TODO rename?
	fn get_finalize_block_hash(&self) -> Option<B::Hash> {
		let try_finalize_block_number = self.client.info().finalized_number.add(One::one());
		self.client.hash(try_finalize_block_number).map_or(None, |v| v)
	}

	pub fn incoming_message(
		&mut self,
		notification: TopicNotification,
	) -> Result<(), HotstuffError> {
		let message: ConsensusMessage<B> =
			Decode::decode(&mut &notification.message[..]).map_err(|e| Other(e.to_string()))?;
		self.consensus_msg_tx.try_send(message).map_err(|e| Other(e.to_string()))
	}
}

impl<B, BE, C, N, S> Future for ConsensusWorker<B, BE, C, N, S>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
{
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let mut gossip_msg_receiver = self
			.network
			.gossip_engine
			.lock()
			.messages_for(ConsensusMessage::<B>::gossip_topic());
		loop {
			match StreamExt::poll_next_unpin(&mut gossip_msg_receiver, cx) {
				Poll::Ready(None) => break,
				Poll::Ready(Some(notification)) => {
					if let Err(e) = self.incoming_message(notification) {
						error!("process incoming message error: {:#?}", e)
					}
				},
				Poll::Pending => {},
			};

			match Future::poll(Pin::new(&mut self.network), cx) {
				Poll::Ready(_) => break,
				Poll::Pending => {},
			}
		}

		Poll::Pending
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

pub fn hotstuff_consensus<Block: BlockT, BE: 'static, C, N, S, SC>(
	network: N,
	link: LinkHalf<Block, C, SC>,
	sync: S,
	hotstuff_protocol_name: ProtocolName,
	key_store: KeystorePtr,
) -> sp_blockchain::Result<impl Future<Output = ()> + Send>
where
	BE: Backend<Block> + 'static,
	N: NetworkT<Block> + Sync + 'static,
	S: SyncingT<Block> + Sync + 'static,
	C: ClientForHotstuff<Block, BE> + 'static,
	C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
{
	let LinkHalf { client, .. } = link;

	let hotstuff_network_bridge =
		HotstuffNetworkBridge::new(network.clone(), sync.clone(), hotstuff_protocol_name);

	Ok(async move {})
}
