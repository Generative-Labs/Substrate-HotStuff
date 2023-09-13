use std::{sync::Arc, marker::PhantomData};

use sc_client_api::{LockImportRun, Finalizer, AuxStore, BlockchainEvents, ExecutorProvider, StorageProvider, TransactionFor, Backend};
use sc_consensus::BlockImport;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{HeaderMetadata, HeaderBackend, Error as ClientError};
use sp_runtime::traits::{Block as BlockT, NumberFor};

use crate::{network_bridge::{Network as NetworkT, Syncing as SyncingT, HotstuffNetworkBridge}, import::HotstuffBlockImport};



/// A trait that includes all the client functionalities hotstuff requires.
/// Ideally this would be a trait alias, we're not there yet.
/// tracking issue <https://github.com/rust-lang/rust/issues/41517>
pub trait ClientForHotstuff<Block, BE>:
	LockImportRun<Block, BE>
	+ Finalizer<Block, BE>
	+ AuxStore
	+ HeaderMetadata<Block, Error = sp_blockchain::Error>
	+ HeaderBackend<Block>
	+ BlockchainEvents<Block>
	+ ProvideRuntimeApi<Block>
	+ ExecutorProvider<Block>
	+ BlockImport<Block, Transaction = TransactionFor<BE, Block>, Error = sp_consensus::Error>
	+ StorageProvider<Block, BE>
where
	BE: Backend<Block>,
	Block: BlockT,
{
}


impl<Block, BE, T> ClientForHotstuff<Block, BE> for T
where
	BE: Backend<Block>,
	Block: BlockT,
	T: LockImportRun<Block, BE>
		+ Finalizer<Block, BE>
		+ AuxStore
		+ HeaderMetadata<Block, Error = sp_blockchain::Error>
		+ HeaderBackend<Block>
		+ BlockchainEvents<Block>
		+ ProvideRuntimeApi<Block>
		+ ExecutorProvider<Block>
		+ BlockImport<Block, Transaction = TransactionFor<BE, Block>, Error = sp_consensus::Error>
		+ StorageProvider<Block, BE>,
{
}

/// Something that one can ask to do a block sync request.
pub(crate) trait BlockSyncRequester<Block: BlockT> {
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

// impl<Block, Network, Syncing> BlockSyncRequester<Block> for HotstuffNetworkBridge<Block, Network, Syncing>
// where
// 	Block: BlockT,
// 	Network: NetworkT<Block>,
// 	Syncing: SyncingT<Block>,
// {
// 	fn set_sync_fork_request(
// 		&self,
// 		peers: Vec<sc_network::PeerId>,
// 		hash: Block::Hash,
// 		number: NumberFor<Block>,
// 	) {
// 		HotstuffNetworkBridge::set_sync_fork_request(self, peers, hash, number)
// 	}
// }

/// Persistent data kept between runs.
pub(crate) struct PersistentData<Block: BlockT> {
    _phantom: Option<PhantomData<Block>>,
}

/// Link between the block importer and the background voter.
pub struct LinkHalf<Block: BlockT, C> {
	client: Arc<C>,
	persistent_data: PersistentData<Block>,
	// voter_commands_rx: TracingUnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
	// justification_sender: GrandpaJustificationSender<Block>,
	// justification_stream: GrandpaJustificationStream<Block>,
	// telemetry: Option<TelemetryHandle>,
}



/// Make block importer and link half necessary to tie the background voter
/// to it.
pub fn block_import<BE, Block: BlockT, Client>(
	client: Arc<Client>,
) -> Result<(HotstuffBlockImport<BE, Block, Client>, LinkHalf<Block, Client>), ClientError>
where
	BE: Backend<Block> + 'static,
	Client: ClientForHotstuff<Block, BE> + 'static,
{

}