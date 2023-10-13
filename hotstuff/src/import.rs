use std::{
	collections::VecDeque,
	marker::PhantomData,
	ops::Add,
	pin::Pin,
	sync::{Arc, Mutex},
	task::{Context, Poll},
};

use futures::{Future, StreamExt};
use sc_client_api::{
	client::{FinalityNotifications, ImportNotifications},
	Backend,
};
use sc_consensus::{
	BlockCheckParams, BlockImport, BlockImportParams, ImportResult, JustificationImport,
};
use sc_network::{PeerId, ReputationChange};
use sp_api::TransactionFor;
use sp_blockchain::BlockStatus;
use sp_consensus::Error as ConsensusError;
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Header as HeaderT, NumberFor, One},
	Justification,
};

use crate::{
	client::ClientForHotstuff,
	primitives::{HotstuffError, HotstuffError::*},
};

// TODO remove finalize_grandpa reference.

// const LOG_TARGET: &str  = "hotstuff";
pub struct HotstuffBlockImport<Backend, Block: BlockT, Client> {
	inner: Arc<Client>,
	backend: PhantomData<Backend>,
	_phantom: PhantomData<Block>,
}

impl<Backend, Block: BlockT, Client> Clone for HotstuffBlockImport<Backend, Block, Client> {
	fn clone(&self) -> Self {
		HotstuffBlockImport {
			inner: self.inner.clone(),
			backend: PhantomData,
			_phantom: PhantomData,
		}
	}
}

impl<Backend, Block: BlockT, Client> HotstuffBlockImport<Backend, Block, Client> {
	pub fn new(inner: Arc<Client>) -> HotstuffBlockImport<Backend, Block, Client> {
		HotstuffBlockImport { inner, backend: PhantomData, _phantom: PhantomData }
	}
}

#[async_trait::async_trait]
impl<BE, Block: BlockT, Client> BlockImport<Block> for HotstuffBlockImport<BE, Block, Client>
where
	// NumberFor<Block>: finality_grandpa::BlockNumberOps,
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	for<'a> &'a Client:
		BlockImport<Block, Error = ConsensusError, Transaction = TransactionFor<Client, Block>>,
	TransactionFor<Client, Block>: 'static,
{
	type Error = ConsensusError;
	type Transaction = TransactionFor<Client, Block>;

	async fn check_block(
		&mut self,
		block: BlockCheckParams<Block>,
	) -> Result<ImportResult, Self::Error> {
		self.inner.check_block(block).await
	}

	async fn import_block(
		&mut self,
		mut block: BlockImportParams<Block, Self::Transaction>,
	) -> Result<ImportResult, Self::Error> {
		let hash = block.post_hash();

		match self.inner.status(hash) {
			Ok(BlockStatus::InChain) => {
				// Strip justifications when re-importing an existing block.
				let _justifications = block.justifications.take();
				return (&*self.inner).import_block(block).await
			},
			Ok(BlockStatus::Unknown) => {},
			Err(e) => return Err(ConsensusError::ClientImport(e.to_string())),
		}

		// if block.with_state() {
		// 	return self.import_state(block).await
		// }

		let import_result = (&*self.inner).import_block(block).await;
		let imported_aux = {
			match import_result {
				Ok(ImportResult::Imported(aux)) => aux,
				Ok(r) => return Ok(r),
				Err(e) => return Err(ConsensusError::ClientImport(e.to_string())),
			}
		};

		// TODO
		Ok(ImportResult::Imported(imported_aux))
	}
}

impl<BE, Block: BlockT, Client> HotstuffBlockImport<BE, Block, Client>
where
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	// NumberFor<Block>: finality_grandpa::BlockNumberOps,
{
	/// Import a block justification and finalize the block.
	///
	/// If `enacts_change` is set to true, then finalizing this block *must*
	/// enact an authority set change, the function will panic otherwise.
	fn import_justification(
		&mut self,
		hash: Block::Hash,
		_number: NumberFor<Block>,
		_justification: Justification,
		_enacts_change: bool,
		_initial_sync: bool,
	) -> Result<(), ConsensusError> {
		let client = self.inner.clone();
		let _status = client.info();

		let _res: Result<(), sp_blockchain::Error> = self.inner.finalize_block(hash, None, true);
		match _res {
			Ok(()) => {
				println!("🔥💃🏻 success finalize_block");
			},
			Err(err) => {
				println!("🔥💃🏻 finalize_block error: {:?}", err);
			},
		}

		Ok(())
	}
}

#[async_trait::async_trait]
impl<BE, Block: BlockT, Client> JustificationImport<Block>
	for HotstuffBlockImport<BE, Block, Client>
where
	// NumberFor<Block>: finality_grandpa::BlockNumberOps,
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
{
	type Error = ConsensusError;

	async fn import_justification(
		&mut self,
		hash: Block::Hash,
		number: NumberFor<Block>,
		justification: Justification,
	) -> Result<(), Self::Error> {
		// this justification was requested by the sync service, therefore we
		// are not sure if it should enact a change or not. it could have been a
		// request made as part of initial sync but that means the justification
		// wasn't part of the block and was requested asynchronously, probably
		// makes sense to log in that case.
		HotstuffBlockImport::import_justification(self, hash, number, justification, false, false)
	}

	async fn on_start(&mut self) -> Vec<(Block::Hash, NumberFor<Block>)> {
		let mut _out = Vec::new();
		let _chain_info = self.inner.info();

		// TODO

		_out
	}
}

// /// Report specifying a reputation change for a given peer.
#[derive(Debug, PartialEq)]
pub(crate) struct PeerReport {
	pub who: PeerId,
	pub cost_benefit: ReputationChange,
}

#[derive(Clone, PartialEq, Eq)]
pub struct BlockInfo<B: BlockT> {
	pub hash: Option<B::Hash>,
	pub number: <B::Header as HeaderT>::Number,
}

// A queue cache the bast block from the client.
pub(crate) struct PendingFinalizeBlockQueue<B: BlockT> {
	import_notification: ImportNotifications<B>,

	// The finalize notification not guaranteed to be fired for every finalize block.
	finalize_notification: FinalityNotifications<B>,

	// A queue cache the block hash and block number which wait for finalize.
	inner: Arc<Mutex<VecDeque<BlockInfo<B>>>>,
}

impl<B: BlockT> PendingFinalizeBlockQueue<B> {
	pub fn new<BE: Backend<B>, C: ClientForHotstuff<B, BE>>(
		client: Arc<C>,
	) -> Result<Self, HotstuffError> {
		let chain_info = client.info();
		let mut block_number = chain_info.finalized_number.add(One::one());
		let mut queue = VecDeque::new();

		while block_number <= chain_info.best_number {
			let block_hash = client
				.block_hash_from_id(&BlockId::Number(block_number))
				.map_err(|e| ClientError(e.to_string()))?;

			queue.push_back((block_hash, block_number));
			block_number += One::one();
		}

		Ok(Self {
			import_notification: client.every_import_notification_stream(),
			finalize_notification: client.finality_notification_stream(),
			inner: Arc::new(Mutex::new(VecDeque::new())),
		})
	}

	pub fn queue(&self) -> Arc<Mutex<VecDeque<BlockInfo<B>>>> {
		self.inner.clone()
	}
}

impl<B: BlockT> Unpin for PendingFinalizeBlockQueue<B> {}

impl<B: BlockT> Future for PendingFinalizeBlockQueue<B> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		loop {
			match StreamExt::poll_next_unpin(&mut self.import_notification, cx) {
				Poll::Ready(None) => break,
				Poll::Ready(Some(notification)) => {
					if !notification.is_new_best {
						continue
					}

					if let Ok(mut pending) = self.inner.lock() {
						log::info!(target: "Hotstuff", "*** push {}", notification.hash);
						pending.push_back(BlockInfo {
							hash: Some(notification.hash),
							number: notification.header.number().clone(),
						});
					}
				},
				Poll::Pending => break,
			}
		}

		loop {
			match StreamExt::poll_next_unpin(&mut self.finalize_notification, cx) {
				Poll::Ready(None) => break,
				Poll::Ready(Some(notification)) =>
					if let Ok(mut pending) = self.inner.lock() {
						let finalized_number = notification.header.number();

						while let Some(elem) = pending.front() {
							if elem.number > *finalized_number {
								break
							}
							if let Some(b) = pending.pop_front() {
								log::info!(target: "Hotstuff", "*** pop {}", b.hash.unwrap());
							}
						}
					},
				Poll::Pending => break,
			}
		}

		Poll::Pending
	}
}
