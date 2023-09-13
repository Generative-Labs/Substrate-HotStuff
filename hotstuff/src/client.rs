use std::{sync::Arc, marker::PhantomData};

use parity_scale_codec::Decode;
use sc_client_api::{LockImportRun, Finalizer, AuxStore, BlockchainEvents, ExecutorProvider, StorageProvider, TransactionFor, Backend, CallExecutor};
use sc_consensus::BlockImport;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{HeaderMetadata, HeaderBackend, Error as ClientError};
use sp_consensus_grandpa::AuthorityList;
use sp_core::traits::CallContext;
use sp_runtime::{traits::{Block as BlockT, NumberFor, Zero}, generic::BlockId};

use crate::{network_bridge::{Network as NetworkT, Syncing as SyncingT, HotstuffNetworkBridge}, import::HotstuffBlockImport, aux_schema, authorities::SharedAuthoritySet};



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
// pub(crate) struct PersistentData<Block: BlockT> {
//     _phantom: Option<PhantomData<Block>>,
// }

/// Link between the block importer and the background voter.
pub struct LinkHalf<Block: BlockT, C, SC> {
	pub client: Arc<C>,
	pub select_chain: Option<PhantomData<SC>>,
	pub(crate) persistent_data: aux_schema::PersistentData<Block>,
	// voter_commands_rx: TracingUnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
	// justification_sender: GrandpaJustificationSender<Block>,
	// justification_stream: GrandpaJustificationStream<Block>,
	
	// telemetry: Option<TelemetryHandle>,
}

impl<Block: BlockT, C, SC> LinkHalf<Block, C, SC> {
	/// Get the shared authority set.
	pub fn shared_authority_set(&self) -> &SharedAuthoritySet<Block::Hash, NumberFor<Block>> {
		&self.persistent_data.authority_set
	}

	// /// Get the receiving end of justification notifications.
	// pub fn justification_stream(&self) -> GrandpaJustificationStream<Block> {
	// 	self.justification_stream.clone()
	// }
}


/// Provider for the Hotstuff authority set configured on the genesis block.
pub trait GenesisAuthoritySetProvider<Block: BlockT> {
	/// Get the authority set at the genesis block.
	fn get(&self) -> Result<AuthorityList, ClientError>;
}


impl<Block: BlockT, E, Client> GenesisAuthoritySetProvider<Block> for Arc<Client>
where
	E: CallExecutor<Block>,
	Client: ExecutorProvider<Block, Executor = E> + HeaderBackend<Block>,
{
	fn get(&self) -> Result<AuthorityList, ClientError> {
		// This implementation uses the Grandpa runtime API instead of reading directly from the
		// `GRANDPA_AUTHORITIES_KEY` as the data may have been migrated since the genesis block of
		// the chain, whereas the runtime API is backwards compatible.
		self.executor()
			.call(
				self.expect_block_hash_from_id(&BlockId::Number(Zero::zero()))?,
				"GrandpaApi_grandpa_authorities",
				&[],
				CallContext::Offchain,
			)
			.and_then(|call_result| {
				Decode::decode(&mut &call_result[..]).map_err(|err| {
					ClientError::CallResultDecode(
						"failed to decode GRANDPA authorities set proof",
						err,
					)
				})
			})
	}
}

/// Make block importer and link half necessary to tie the background voter
/// to it.
pub fn block_import<BE, Block: BlockT, Client, SC>(
	client: Arc<Client>,
	genesis_authorities_provider: &dyn GenesisAuthoritySetProvider<Block>,
) -> Result<(HotstuffBlockImport<BE, Block, Client>, LinkHalf<Block, Client, SC>), ClientError>
where
	BE: Backend<Block> + 'static,
	Client: ClientForHotstuff<Block, BE> + 'static,
{
	let chain_info = client.info();
	let genesis_hash = chain_info.genesis_hash;

	let persistent_data =
		aux_schema::load_persistent(&*client, genesis_hash, <NumberFor<Block>>::zero(),
			move || {
				let authorities = genesis_authorities_provider.get()?;

				Ok(authorities)
			}
	)?;

	Ok((
		HotstuffBlockImport::new(
			client.clone(),
		),
		LinkHalf {
			client: client,
			select_chain: None,
			persistent_data: persistent_data,
		},
	))
}
