#![allow(clippy::type_complexity)]
use std::{marker::PhantomData, sync::Arc};

use parity_scale_codec::Decode;

use sc_client_api::{
	AuxStore, Backend, BlockchainEvents, CallExecutor, ExecutorProvider, Finalizer, LockImportRun,
	StorageProvider, TransactionFor,
};
use sc_consensus::BlockImport;
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{Error as ClientError, HeaderBackend, HeaderMetadata};
use sp_consensus_hotstuff::AuthorityId;
use sp_core::traits::CallContext;
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, NumberFor, Zero},
};

use crate::{authorities::SharedAuthoritySet, aux_schema, import::HotstuffBlockImport};

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

/// Link between the block importer and the background voter.
pub struct LinkHalf<Block: BlockT, C, SC> {
	pub client: Arc<C>,
	pub select_chain: Option<PhantomData<SC>>,
	pub(crate) persistent_data: aux_schema::PersistentData<Block>,
	pub import_block_rx: TracingUnboundedReceiver<(Block::Hash, NumberFor<Block>)>,
}

impl<Block: BlockT, C, SC> LinkHalf<Block, C, SC> {
	/// Get the shared authority set.
	pub fn shared_authority_set(&self) -> &SharedAuthoritySet<Block::Hash, NumberFor<Block>> {
		&self.persistent_data.authority_set
	}
}

/// Provider for the Hotstuff authority set configured on the genesis block.
pub trait GenesisAuthoritySetProvider<Block: BlockT> {
	/// Get the authority set at the genesis block.
	fn get(&self) -> Result<Vec<AuthorityId>, ClientError>;
}

impl<Block: BlockT, E, Client> GenesisAuthoritySetProvider<Block> for Arc<Client>
where
	E: CallExecutor<Block>,
	Client: ExecutorProvider<Block, Executor = E> + HeaderBackend<Block>,
{
	fn get(&self) -> Result<Vec<AuthorityId>, ClientError> {
		// This implementation uses the Grandpa runtime API instead of reading directly from the
		// `GRANDPA_AUTHORITIES_KEY` as the data may have been migrated since the genesis block of
		// the chain, whereas the runtime API is backwards compatible.
		self.executor()
			.call(
				self.expect_block_hash_from_id(&BlockId::Number(Zero::zero()))?,
				"HotstuffApi_authorities",
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

	let persistent_data = aux_schema::load_persistent(
		&*client,
		genesis_hash,
		<NumberFor<Block>>::zero(),
		move || {
			let authorities = genesis_authorities_provider.get()?;

			// TODO just for dev.
			let authorities =
				authorities.iter().map(|p| (p.clone(), 1)).collect::<Vec<(AuthorityId, u64)>>();

			Ok(authorities)
		},
	)?;

	let (import_block_tx, import_block_rx) =
		tracing_unbounded::<(Block::Hash, NumberFor<Block>)>("hotstuff_block_import_ch", 200);

	Ok((
		HotstuffBlockImport::new(client.clone(), import_block_tx),
		LinkHalf { client, select_chain: None, persistent_data, import_block_rx },
	))
}
