use std::{marker::PhantomData, sync::Arc};

use sc_client_api::Backend;
use sc_consensus::{
	BlockCheckParams, BlockImport, BlockImportParams, ImportResult, JustificationImport,
};
use sc_network::{PeerId, ReputationChange};
use sc_utils::mpsc::TracingUnboundedSender;
use sp_api::TransactionFor;
use sp_blockchain::BlockStatus;
use sp_consensus::Error as ConsensusError;
use sp_consensus_grandpa::GrandpaApi;
use sp_runtime::{
	traits::{Block as BlockT, Header as HeaderT, NumberFor},
	Justification,
};

use crate::client::ClientForHotstuff;

// TODO remove finalize_grandpa reference.

// const LOG_TARGET: &str  = "hotstuff";
pub struct HotstuffBlockImport<Backend, Block: BlockT, Client> {
	inner: Arc<Client>,
	block_import_sender: TracingUnboundedSender<(Block::Hash, NumberFor<Block>)>,
	backend: PhantomData<Backend>,
	_phantom: PhantomData<Block>,
}

impl<Backend, Block: BlockT, Client> Clone for HotstuffBlockImport<Backend, Block, Client> {
	fn clone(&self) -> Self {
		HotstuffBlockImport {
			inner: self.inner.clone(),
			block_import_sender: self.block_import_sender.clone(),
			backend: PhantomData,
			_phantom: PhantomData,
		}
	}
}

impl<Backend, Block: BlockT, Client> HotstuffBlockImport<Backend, Block, Client> {
	pub fn new(
		inner: Arc<Client>,
		block_import_sender: TracingUnboundedSender<(Block::Hash, NumberFor<Block>)>,
	) -> HotstuffBlockImport<Backend, Block, Client> {
		HotstuffBlockImport {
			inner,
			block_import_sender,
			backend: PhantomData,
			_phantom: PhantomData,
		}
	}
}

#[async_trait::async_trait]
impl<BE, Block: BlockT, Client> BlockImport<Block> for HotstuffBlockImport<BE, Block, Client>
where
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	Client::Api: GrandpaApi<Block>,
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
		let number = *block.header.number();

		println!("🔥 >>> import block: hash: {:?} number:{:?}", hash, number);
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

		if imported_aux.is_new_best {
			match self.block_import_sender.unbounded_send((hash, number)) {
				Ok(_) => {},
				Err(e) => return Err(ConsensusError::ClientImport(e.to_string())),
			}
		}

		// TODO
		Ok(ImportResult::Imported(imported_aux))
	}
}

impl<BE, Block: BlockT, Client> HotstuffBlockImport<BE, Block, Client>
where
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
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

		println!("🔥💃🏻 import_justification finalize_block start");
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
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
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
