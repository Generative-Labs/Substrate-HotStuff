use futures::prelude::*;
use sc_consensus_grandpa::{
	BlockNumberOps, ClientForGrandpa, Config, LinkHalf, SharedVoterState, VotingRule,
};
use std::{marker::PhantomData, sync::Arc};

use sc_telemetry::TelemetryHandle;
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use sp_consensus::SelectChain;
use sp_consensus_grandpa::GrandpaApi;
use sp_runtime::traits::{Block as BlockT, NumberFor};

// use prometheus_endpoint::{PrometheusError, Registry};

use sc_client_api::backend::Backend;

pub struct FinalityBlock<Block: BlockT, BE, Client> {
	pub client: Arc<Client>,
	// authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
	pub _authority_set: PhantomData<Block>,
	pub _phantom: PhantomData<BE>,
}

pub struct HotstuffFinalityParam<Client, Block: BlockT> {
	pub client: Arc<Client>,
	pub block: Option<Block>,
}

/// Parameters used to run Grandpa.
pub struct GrandpaParams<Block: BlockT, C, SC, VR> {
	/// Configuration for the GRANDPA service.
	pub client: Arc<C>,
	pub config: Config,
	/// A link to the block import worker.
	pub link: LinkHalf<Block, C, SC>,
	/// The Network instance.
	///
	/// It is assumed that this network will feed us Grandpa notifications. When using the
	/// `sc_network` crate, it is assumed that the Grandpa notifications protocol has been passed
	/// to the configuration of the networking. See [`grandpa_peers_set_config`].

	/// A voting rule used to potentially restrict target votes.
	pub voting_rule: VR,
	/// The prometheus metrics registry.
	pub prometheus_registry: Option<prometheus_endpoint::Registry>,
	/// The voter state is exposed at an RPC endpoint.
	pub shared_voter_state: SharedVoterState,
	/// TelemetryHandle instance.
	pub telemetry: Option<TelemetryHandle>,
	/// Offchain transaction pool factory.
	///
	/// This will be used to create an offchain transaction pool instance for sending an
	/// equivocation report from the runtime.
	pub offchain_tx_pool_factory: OffchainTransactionPoolFactory<Block>,
}

/// Run a HOTSTUFF voter as a task. Provide configuration and a link to a
/// block import worker that has already been instantiated with `block_import`.
#[allow(unused)]
pub fn run_hotstuff_voter<Block: BlockT, BE: 'static, C, SC, VR>(
	grandpa_params: GrandpaParams<Block, C, SC, VR>,
) -> sp_blockchain::Result<impl Future<Output = ()> + Send>
where
	BE: Backend<Block> + 'static,
	SC: SelectChain<Block> + 'static,
	VR: VotingRule<Block, C> + Clone + 'static,
	NumberFor<Block>: BlockNumberOps,
	C: ClientForGrandpa<Block, BE> + 'static,
	C::Api: GrandpaApi<Block>,
{
	let GrandpaParams {
		client,
		mut config,
		link,
		voting_rule,
		prometheus_registry,
		shared_voter_state,
		telemetry,
		offchain_tx_pool_factory,
	} = grandpa_params;

	config.observer_enabled = false;

	let future1 = future::ready(());
	let future2 = future::ready(());

	let voter_work = future1.then(|_| future::pending::<()>());

	let telemetry_task = future2.then(|_| future::pending::<()>());

	println!("hello 🔥 run hotstuff");
	Ok(future::select(voter_work, telemetry_task).map(drop))
}

pub fn run_hotstuff_voter_test<BE, Block: BlockT, Client>(
	client: Arc<Client>,
) -> sp_blockchain::Result<impl Future<Output = ()> + Send>
where
	BE: Backend<Block> + 'static,
	Client: ClientForGrandpa<Block, BE> + 'static,
{
	let block_hash = client.info().genesis_hash;
	println!("hash>>>: {}", block_hash);

	let future1 = future::ready(());
	let future2 = future::ready(());

	let res = client.finalize_block(block_hash, None, true);
	match res {
		Ok(()) => {
			println!("suecess");
		},
		Err(err) => {
			println!("error: {:?}", err);
		},
	}

	let voter_work = future1.then(|_| future::pending::<()>());

	let telemetry_task = future2.then(|_| future::pending::<()>());

	// println!("hello 🔥 run test");
	Ok(future::select(voter_work, telemetry_task).map(drop))
}
