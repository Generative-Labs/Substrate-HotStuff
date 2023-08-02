use std::{marker::PhantomData, pin::Pin, sync::Arc, time::Duration};

use crate::environment::VoterSetState;
use authorities::{AuthoritySet, AuthoritySetChanges, SharedAuthoritySet};
use aux_schema::PersistentData;
use communication::{Network as NetworkT, NetworkBridge, Round};
use environment::Environment;
use finality_tendermint::{messages, voter, BlockNumberOps, Error as TendermintError, VoterSet};
use futures::{future, prelude::*, Future, Sink, SinkExt, Stream, StreamExt, TryStreamExt};
use log::{debug, error, info};
use parity_scale_codec::{Decode, Encode};
use parking_lot::RwLock;
use prometheus_endpoint::{PrometheusError, Registry};
use sc_client_api::{
	AuxStore, Backend, BlockchainEvents, CallExecutor, ExecutionStrategy, ExecutorProvider,
	Finalizer, HeaderBackend, LockImportRun, StorageProvider, TransactionFor,
};
use sc_consensus::BlockImport;
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_DEBUG, CONSENSUS_INFO};
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{Error as ClientError, HeaderMetadata};
use sp_consensus::SelectChain;
use sp_finality_tendermint::{
	AuthorityId, AuthorityList, AuthoritySignature, SetId, TendermintApi,
};
use sp_keystore::{SyncCryptoStore, SyncCryptoStorePtr};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Header as HeaderT, NumberFor, Zero},
};

use sp_runtime::RuntimeAppPublic;

use std::{
	fmt, io,
	task::{Context, Poll},
};

// utility logging macro that takes as first argument a conditional to
// decide whether to log under debug or info level (useful to restrict
// logging under initial sync).
macro_rules! afp_log {
	($condition:expr, $($msg: expr),+ $(,)?) => {
		{
			let log_level = if $condition {
				log::Level::Debug
			} else {
				log::Level::Info
			};

			log::log!(target: "afp", log_level, $($msg),+);
		}
	};
}

pub(crate) mod authorities;
pub(crate) mod aux_schema;
pub(crate) mod communication;
pub(crate) mod environment;
pub(crate) mod import;
pub(crate) mod justification;
pub(crate) mod notification;
pub(crate) mod until_imported;
// pub mod warp_proof;

pub use communication::tendermint_protocol_name::standard_name as protocol_standard_name;
pub use import::TendermintBlockImport;
pub use notification::{TendermintJustificationSender, TendermintJustificationStream};
use until_imported::UntilGlobalMessageBlocksImported;

/// A Tendermint message for a substrate chain.
pub type Message<Block> = messages::Message<NumberFor<Block>, <Block as BlockT>::Hash>;

/// A signed message
pub type SignedMessage<Block> = messages::SignedMessage<
	NumberFor<Block>,
	<Block as BlockT>::Hash,
	AuthoritySignature,
	AuthorityId,
>;

/// A preprepare message for this chain's block type.
pub type Proposal<Block> = messages::Proposal<NumberFor<Block>, <Block as BlockT>::Hash>;
/// A prepare message for this chain's block type.
pub type Prevote<Block> = messages::Prevote<NumberFor<Block>, <Block as BlockT>::Hash>;
/// A commit message for this chain's block type.
pub type Precommit<Block> = messages::Precommit<NumberFor<Block>, <Block as BlockT>::Hash>;
/// A finalized commit message for this chain's block type.
pub type FinalizedCommit<Block> = messages::FinalizedCommit<
	NumberFor<Block>,
	<Block as BlockT>::Hash,
	AuthoritySignature,
	AuthorityId,
>;

// pub type RoundChange = messages::RoundChange<AuthorityId>;

/// FIXME:
#[derive(Debug, Encode, Decode)]
pub enum GlobalMessage {
	// RoundChange(RoundChange),
	Empty,
}

/// a compact commit message for this chain's block type.
pub type CompactCommit<Block> = messages::CompactCommit<
	<Block as BlockT>::Hash,
	NumberFor<Block>,
	AuthoritySignature,
	AuthorityId,
>;

/// A global communication input stream for commits and catch up messages. Not
/// exposed publicly, used internally to simplify types in the communication
/// layer.
type GlobalCommunicationIn<Block> = messages::GlobalMessageIn<
	<Block as BlockT>::Hash,
	NumberFor<Block>,
	AuthoritySignature,
	AuthorityId,
>;

/// Global communication input stream for commits and catch up messages, with
/// the hash type not being derived from the block, useful for forcing the hash
/// to some type (e.g. `H256`) when the compiler can't do the inference.
// type GlobalCommunicationInH<Block, H> =
// messages::voter::GlobalMessageIn<H, NumberFor<Block>, AuthoritySignature, AuthorityId>;

/// Global communication sink for commits with the hash type not being derived
/// from the block, useful for forcing the hash to some type (e.g. `H256`) when
/// the compiler can't do the inference.
type GlobalCommunicationOut<Block> = messages::GlobalMessageOut<
	<Block as BlockT>::Hash,
	NumberFor<Block>,
	AuthoritySignature,
	AuthorityId,
>;
// type GlobalCommunicationOutH<Block, H> =
// 	messages::voter::GlobalMessageOut<H, NumberFor<Block>, AuthoritySignature, AuthorityId>;

/// Shared voter state for querying.
#[derive(Clone)]
pub struct SharedVoterState<Block: BlockT> {
	inner: Arc<
		RwLock<
			Option<
				Box<
					dyn finality_tendermint::voter::report::VoterStateT<
							<Block as BlockT>::Hash,
							AuthorityId,
						> + Sync
						+ Send,
				>,
			>,
		>,
	>,
}

impl<Block: BlockT> SharedVoterState<Block> {
	/// Create a new empty `SharedVoterState` instance.
	pub fn empty() -> Self {
		Self { inner: Arc::new(RwLock::new(None)) }
	}

	fn reset(
		&self,
		voter_state: Box<dyn voter::report::VoterStateT<Block::Hash, AuthorityId> + Sync + Send>,
	) -> Option<()> {
		let mut shared_voter_state = self.inner.try_write_for(Duration::from_secs(1))?;

		*shared_voter_state = Some(voter_state);
		Some(())
	}

	// TODO: FKY: Get the inner `VoterState` instance.
	pub fn voter_state(&self) -> Option<voter::report::VoterState<Block::Hash, AuthorityId>> {
		self.inner.read().as_ref().map(|vs| vs.get())
	}
}

/// Link between the block importer and the background voter.
pub struct LinkHalf<Block: BlockT, C, SC> {
	client: Arc<C>,
	select_chain: SC,
	persistent_data: PersistentData<Block>,
	voter_commands_rx: TracingUnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
	justification_sender: TendermintJustificationSender<Block>,
	justification_stream: TendermintJustificationStream<Block>,
	telemetry: Option<TelemetryHandle>,
}

impl<Block: BlockT, C, SC> LinkHalf<Block, C, SC> {
	/// Get the shared authority set.
	pub fn shared_authority_set(&self) -> &SharedAuthoritySet<Block::Hash, NumberFor<Block>> {
		&self.persistent_data.authority_set
	}
}

/// Provider for the Grandpa authority set configured on the genesis block.
pub trait GenesisAuthoritySetProvider<Block: BlockT> {
	/// Get the authority set at the genesis block.
	fn get(&self) -> Result<AuthorityList, ClientError>;
}

impl<Block: BlockT, E> GenesisAuthoritySetProvider<Block>
	for Arc<dyn ExecutorProvider<Block, Executor = E>>
where
	E: CallExecutor<Block>,
{
	fn get(&self) -> Result<AuthorityList, ClientError> {
		// NOTE: This implementation uses the Grandpa runtime API instead of reading directly from
		// the `GRANDPA_AUTHORITIES_KEY` as the data may have been migrated since the genesis block
		// of the chain, whereas the runtime API is backwards compatible.
		self.executor()
			.call(
				&BlockId::Number(Zero::zero()),
				"TendermintApi_tendermint_authorities",
				&[],
				ExecutionStrategy::NativeElseWasm,
				None,
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
	select_chain: SC,
	telemetry: Option<TelemetryHandle>,
) -> Result<(TendermintBlockImport<BE, Block, Client, SC>, LinkHalf<Block, Client, SC>), ClientError>
where
	SC: SelectChain<Block>,
	BE: Backend<Block> + 'static,
	Client: ClientForTendermint<Block, BE> + 'static,
{
	block_import_with_authority_set_hard_forks(
		client,
		genesis_authorities_provider,
		select_chain,
		Default::default(),
		telemetry,
	)
}

/// A descriptor for an authority set hard fork. These are authority set changes
/// that are not signalled by the runtime and instead are defined off-chain
/// (hence the hard fork).
pub struct AuthoritySetHardFork<Block: BlockT> {
	/// The new authority set id.
	pub set_id: SetId,
	/// The block hash and number at which the hard fork should be applied.
	pub block: (Block::Hash, NumberFor<Block>),
	/// The authorities in the new set.
	pub authorities: AuthorityList,
	/// The latest block number that was finalized before this authority set
	/// hard fork. When defined, the authority set change will be forced, i.e.
	/// the node won't wait for the block above to be finalized before enacting
	/// the change, and the given finalized number will be used as a base for
	/// voting.
	pub last_finalized: Option<NumberFor<Block>>,
}
/// Make block importer and link half necessary to tie the background voter to
/// it. A vector of authority set hard forks can be passed, any authority set
/// change signaled at the given block (either already signalled or in a further
/// block when importing it) will be replaced by a standard change with the
/// given static authorities.
pub fn block_import_with_authority_set_hard_forks<BE, Block: BlockT, Client, SC>(
	client: Arc<Client>,
	genesis_authorities_provider: &dyn GenesisAuthoritySetProvider<Block>,
	select_chain: SC,
	authority_set_hard_forks: Vec<AuthoritySetHardFork<Block>>,
	telemetry: Option<TelemetryHandle>,
) -> Result<(TendermintBlockImport<BE, Block, Client, SC>, LinkHalf<Block, Client, SC>), ClientError>
where
	SC: SelectChain<Block>,
	BE: Backend<Block> + 'static,
	Client: ClientForTendermint<Block, BE> + 'static,
{
	let chain_info = client.info();
	let genesis_hash = chain_info.genesis_hash;

	let persistent_data =
		aux_schema::load_persistent(&*client, genesis_hash, <NumberFor<Block>>::zero(), {
			let telemetry = telemetry.clone();
			move || {
				let authorities = genesis_authorities_provider.get()?;
				telemetry!(
					telemetry;
					CONSENSUS_DEBUG;
					"afp.loading_authorities";
					"authorities_len" => ?authorities.len()
				);
				Ok(authorities)
			}
		})?;

	let (voter_commands_tx, voter_commands_rx) = tracing_unbounded("mpsc_tendermint_voter_command");

	let (justification_sender, justification_stream) = TendermintJustificationStream::channel();

	// create pending change objects with 0 delay for each authority set hard fork.
	let authority_set_hard_forks = authority_set_hard_forks
		.into_iter()
		.map(|fork| {
			let delay_kind = if let Some(last_finalized) = fork.last_finalized {
				authorities::DelayKind::Best { median_last_finalized: last_finalized }
			} else {
				authorities::DelayKind::Finalized
			};

			(
				fork.set_id,
				authorities::PendingChange {
					next_authorities: fork.authorities,
					delay: Zero::zero(),
					canon_hash: fork.block.0,
					canon_height: fork.block.1,
					delay_kind,
				},
			)
		})
		.collect();

	Ok((
		TendermintBlockImport::new(
			client.clone(),
			select_chain.clone(),
			persistent_data.authority_set.clone(),
			voter_commands_tx,
			authority_set_hard_forks,
			justification_sender.clone(),
			telemetry.clone(),
		),
		LinkHalf {
			client,
			select_chain,
			persistent_data,
			voter_commands_rx,
			justification_sender,
			justification_stream,
			telemetry,
		},
	))
}

fn global_communication<BE, Block: BlockT, C, N>(
	set_id: SetId,
	voters: &Arc<VoterSet<AuthorityId>>,
	client: Arc<C>,
	network: &NetworkBridge<Block, N>,
	keystore: Option<&SyncCryptoStorePtr>,
	metrics: Option<until_imported::Metrics>,
) -> (
	impl Stream<
		Item = Result<GlobalCommunicationIn<Block>, CommandOrError<Block::Hash, NumberFor<Block>>>,
	>,
	impl Sink<GlobalCommunicationOut<Block>, Error = CommandOrError<Block::Hash, NumberFor<Block>>>,
)
where
	BE: Backend<Block> + 'static,
	C: ClientForTendermint<Block, BE> + 'static,
	N: NetworkT<Block>,
	NumberFor<Block>: BlockNumberOps,
{
	let is_voter = local_authority_id(voters, keystore).is_some();

	// verification stream
	let (global_in, global_out) =
		network.global_communication(communication::SetId(set_id), voters.clone(), is_voter);

	// block commit and catch up messages until relevant blocks are imported.
	let global_in = UntilGlobalMessageBlocksImported::new(
		client.import_notification_stream(),
		network.clone(),
		client.clone(),
		global_in,
		"global",
		metrics,
	);

	let global_in = global_in.map_err(CommandOrError::from);
	let global_out = global_out.sink_map_err(CommandOrError::from);

	(global_in, global_out)
}

struct Metrics {
	environment: environment::Metrics,
	until_imported: until_imported::Metrics,
}

impl Metrics {
	fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Metrics {
			environment: environment::Metrics::register(registry)?,
			until_imported: until_imported::Metrics::register(registry)?,
		})
	}
}

/// Futures that powers the voter.
#[must_use]
struct VoterWork<B, Block: BlockT, C, N: NetworkT<Block>, SC> {
	voter: Pin<
		Box<dyn Future<Output = Result<(), CommandOrError<Block::Hash, NumberFor<Block>>>> + Send>,
	>,
	shared_voter_state: SharedVoterState<Block>,
	env: Arc<Environment<B, Block, C, N, SC>>,
	voter_commands_rx: TracingUnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
	network: NetworkBridge<Block, N>,
	telemetry: Option<TelemetryHandle>,
	/// Prometheus metrics.
	metrics: Option<Metrics>,
}

impl<B, Block, C, N, SC> VoterWork<B, Block, C, N, SC>
where
	Block: BlockT,
	B: Backend<Block> + 'static,
	C: ClientForTendermint<Block, B> + 'static,
	C::Api: TendermintApi<Block>,
	N: NetworkT<Block> + Sync,
	NumberFor<Block>: BlockNumberOps,
	SC: SelectChain<Block> + 'static,
{
	fn new(
		client: Arc<C>,
		config: Config,
		network: NetworkBridge<Block, N>,
		select_chain: SC,
		persistent_data: PersistentData<Block>,
		voter_commands_rx: TracingUnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
		prometheus_registry: Option<prometheus_endpoint::Registry>,
		shared_voter_state: SharedVoterState<Block>,
		justification_sender: TendermintJustificationSender<Block>,
		telemetry: Option<TelemetryHandle>,
	) -> Self {
		let metrics = match prometheus_registry.as_ref().map(Metrics::register) {
			Some(Ok(metrics)) => Some(metrics),
			Some(Err(e)) => {
				debug!(target: "afp", "Failed to register metrics: {:?}", e);
				None
			},
			None => None,
		};

		let voters = persistent_data.authority_set.current_authorities();
		let env = Arc::new(Environment {
			client,
			select_chain,
			voters: Arc::new(voters),
			config,
			network: network.clone(),
			set_id: persistent_data.authority_set.set_id(),
			authority_set: persistent_data.authority_set.clone(),
			voter_set_state: persistent_data.set_state,
			metrics: metrics.as_ref().map(|m| m.environment.clone()),
			justification_sender: Some(justification_sender),
			telemetry: telemetry.clone(),
			_phantom: PhantomData,
		});

		let mut work = VoterWork {
			// `voter` is set to a temporary value and replaced below when
			// calling `rebuild_voter`.
			voter: Box::pin(future::pending()),
			shared_voter_state,
			env,
			voter_commands_rx,
			network,
			telemetry,
			metrics,
		};
		work.rebuild_voter();
		work
	}

	/// Rebuilds the `self.voter` field using the current authority set
	/// state. This method should be called when we know that the authority set
	/// has changed (e.g. as signalled by a voter command).
	fn rebuild_voter(&mut self) {
		debug!(target: "afp", "{}: Starting new voter with set ID {}", self.env.config.name(), self.env.set_id);

		let maybe_authority_id =
			local_authority_id(&self.env.voters, self.env.config.keystore.as_ref());
		let authority_id = maybe_authority_id.map_or("<unknown>".into(), |s| s.to_string());

		telemetry!(
			self.telemetry;
			CONSENSUS_DEBUG;
			"afp.starting_new_voter";
			"name" => ?self.env.config.name(),
			"set_id" => ?self.env.set_id,
			"authority_id" => authority_id,
		);

		let chain_info = self.env.client.info();

		let authorities = self.env.voters.iter().map(|id| id.to_string()).collect::<Vec<_>>();

		let authorities = serde_json::to_string(&authorities).expect(
			"authorities is always at least an empty vector; elements are always of type string; qed.",
		);

		telemetry!(
			self.telemetry;
			CONSENSUS_INFO;
			"afp.authority_set";
			"number" => ?chain_info.finalized_number,
			"hash" => ?chain_info.finalized_hash,
			"authority_id" => authority_id,
			"authority_set_id" => ?self.env.set_id,
			"authorities" => authorities,
		);

		match &*self.env.voter_set_state.read() {
			VoterSetState::Live { completed_rounds, .. } => {
				let last_finalized = (chain_info.finalized_hash, chain_info.finalized_number);

				let global_comms = global_communication(
					self.env.set_id,
					&self.env.voters,
					self.env.client.clone(),
					&self.env.network,
					self.env.config.keystore.as_ref(),
					self.metrics.as_ref().map(|m| m.until_imported.clone()),
				);

				let last_completed_round = completed_rounds.last();

				afp_log!(
					true,
					"last_finalized: {:?}, last_completed_round: {:?}.",
					last_finalized,
					last_completed_round
				);

				let mut voter = voter::Voter::new(
					self.env.clone(),
					Box::new(global_comms.0),
					Box::pin(global_comms.1),
					(*self.env.voters).clone(),
					last_completed_round.base,
				);

				// Repoint shared_voter_state so that the RPC endpoint can query the state
				if self.shared_voter_state.reset(voter.voter_state()).is_none() {
					info!(target: "afp",
						"Timed out trying to update shared GRANDPA voter state. \
						RPC endpoints may return stale data."
					);
				}

				self.voter = Box::pin(async move {
					// TODO: Result<>
					voter.start().await;
					afp_log!(true, "async voter exit.");
					Ok(())
				});
			},
			VoterSetState::Paused { .. } => self.voter = Box::pin(future::pending()),
		};
	}

	fn handle_voter_command(
		&mut self,
		command: VoterCommand<Block::Hash, NumberFor<Block>>,
	) -> Result<(), Error> {
		match command {
			VoterCommand::ChangeAuthorities(new) => {
				let voters: Vec<String> =
					new.authorities.iter().map(move |a| format!("{}", a)).collect();
				telemetry!(
					self.telemetry;
					CONSENSUS_INFO;
					"afp.voter_command_change_authorities";
					"number" => ?new.canon_number,
					"hash" => ?new.canon_hash,
					"voters" => ?voters,
					"set_id" => ?new.set_id,
				);

				self.env.update_voter_set_state(|_| {
					// start the new authority set using the block where the
					// set changed (not where the signal happened!) as the base.
					let set_state = VoterSetState::live(
						new.set_id,
						&*self.env.authority_set.inner(),
						(new.canon_hash, new.canon_number),
					);

					aux_schema::write_voter_set_state(&*self.env.client, &set_state)?;
					Ok(Some(set_state))
				})?;

				let voters = Arc::new(VoterSet::new(new.authorities.to_vec()).expect(
					"new authorities come from pending change; pending change comes from \
					 `AuthoritySet`; `AuthoritySet` validates authorities is non-empty and \
					 weights are non-zero; qed.",
				));

				self.env = Arc::new(Environment {
					voters,
					set_id: new.set_id,
					voter_set_state: self.env.voter_set_state.clone(),
					client: self.env.client.clone(),
					select_chain: self.env.select_chain.clone(),
					config: self.env.config.clone(),
					authority_set: self.env.authority_set.clone(),
					network: self.env.network.clone(),
					metrics: self.env.metrics.clone(),
					justification_sender: self.env.justification_sender.clone(),
					telemetry: self.telemetry.clone(),
					_phantom: PhantomData,
				});

				self.rebuild_voter();
				Ok(())
			},
			VoterCommand::Pause(reason) => {
				info!(target: "afp", "Pausing old validator set: {}", reason);

				// not racing because old voter is shut down.
				self.env.update_voter_set_state(|voter_set_state| {
					let completed_rounds = voter_set_state.completed_rounds();
					let set_state = VoterSetState::Paused { completed_rounds };

					aux_schema::write_voter_set_state(&*self.env.client, &set_state)?;
					Ok(Some(set_state))
				})?;

				self.rebuild_voter();
				Ok(())
			},
		}
	}
}

impl<B, Block, C, N, SC> Future for VoterWork<B, Block, C, N, SC>
where
	Block: BlockT,
	B: Backend<Block> + 'static,
	N: NetworkT<Block> + Sync,
	NumberFor<Block>: BlockNumberOps,
	SC: SelectChain<Block> + 'static,
	C: ClientForTendermint<Block, B> + 'static,
	C::Api: TendermintApi<Block>,
{
	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		match Future::poll(Pin::new(&mut self.voter), cx) {
			Poll::Pending => {},
			Poll::Ready(Ok(())) => {
				// voters don't conclude naturally
				return Poll::Ready(Err(Error::Safety(
					"finality-tendermint inner voter has concluded.".into(),
				)))
			},
			Poll::Ready(Err(CommandOrError::Error(e))) => {
				// return inner observer error
				return Poll::Ready(Err(e))
			},
			Poll::Ready(Err(CommandOrError::VoterCommand(command))) => {
				// some command issued internally
				self.handle_voter_command(command)?;
				cx.waker().wake_by_ref();
			},
		}

		match Stream::poll_next(Pin::new(&mut self.voter_commands_rx), cx) {
			Poll::Pending => {},
			Poll::Ready(None) => {
				// the `voter_commands_rx` stream should never conclude since it's never closed.
				return Poll::Ready(Err(Error::Safety("`voter_commands_rx` was closed.".into())))
			},
			Poll::Ready(Some(command)) => {
				// some command issued externally
				self.handle_voter_command(command)?;
				cx.waker().wake_by_ref();
			},
		}

		Future::poll(Pin::new(&mut self.network), cx)
	}
}
/// Parameters used to run Grandpa.
pub struct TendermintParams<Block: BlockT, C, N, SC> {
	/// Configuration for the GRANDPA service.
	pub config: Config,
	/// A link to the block import worker.
	pub link: LinkHalf<Block, C, SC>,
	/// The Network instance.
	///
	/// It is assumed that this network will feed us Grandpa notifications. When using the
	/// `sc_network` crate, it is assumed that the Grandpa notifications protocol has been passed
	/// to the configuration of the networking. See [`grandpa_peers_set_config`].
	pub network: N,
	/// A voting rule used to potentially restrict target votes.
	// pub voting_rule: VR,
	/// The prometheus metrics registry.
	pub prometheus_registry: Option<prometheus_endpoint::Registry>,
	/// The voter state is exposed at an RPC endpoint.
	pub shared_voter_state: SharedVoterState<Block>,
	/// TelemetryHandle instance.
	pub telemetry: Option<TelemetryHandle>,
}

/// Returns the configuration value to put in
/// [`sc_network::config::NetworkConfiguration::extra_sets`].
/// For standard protocol name see [`crate::protocol_standard_name`].
pub fn tendermint_peers_set_config(
	protocol_name: std::borrow::Cow<'static, str>,
) -> sc_network::config::NonDefaultSetConfig {
	sc_network::config::NonDefaultSetConfig {
		notifications_protocol: protocol_name,
		fallback_names: Vec::new(),
		// Notifications reach ~256kiB in size at the time of writing on Kusama and Polkadot.
		max_notification_size: 1024 * 1024,
		set_config: sc_network::config::SetConfig {
			in_peers: 0,
			out_peers: 0,
			reserved_nodes: Vec::new(),
			non_reserved_mode: sc_network::config::NonReservedPeerMode::Deny,
		},
	}
}

/// Run a GRANDPA voter as a task. Provide configuration and a link to a
/// block import worker that has already been instantiated with `block_import`.
pub fn run_tendermint_voter<Block: BlockT, BE: 'static, C, N, SC>(
	tendermint_params: TendermintParams<Block, C, N, SC>,
) -> sp_blockchain::Result<impl Future<Output = ()> + Send>
where
	Block::Hash: Ord,
	BE: Backend<Block> + 'static,
	N: NetworkT<Block> + Sync + 'static,
	SC: SelectChain<Block> + 'static,
	// VR: VotingRule<Block, C> + Clone + 'static,
	NumberFor<Block>: BlockNumberOps,
	C: ClientForTendermint<Block, BE> + 'static,
	C::Api: TendermintApi<Block>,
{
	let TendermintParams {
		mut config,
		link,
		network,
		prometheus_registry,
		shared_voter_state,
		telemetry,
	} = tendermint_params;

	// NOTE: we have recently removed `run_tendermint_observer` from the public
	// API, I felt it is easier to just ignore this field rather than removing
	// it from the config temporarily. This should be removed after #5013 is
	// fixed and we re-add the observer to the public API.
	config.observer_enabled = false;

	let LinkHalf {
		client,
		select_chain,
		persistent_data,
		voter_commands_rx,
		justification_sender,
		justification_stream: _,
		telemetry: _,
	} = link;

	let network = NetworkBridge::new(
		network,
		config.clone(),
		persistent_data.set_state.clone(),
		prometheus_registry.as_ref(),
		telemetry.clone(),
	);

	let conf = config.clone();
	let telemetry_task =
		if let Some(telemetry_on_connect) = telemetry.as_ref().map(|x| x.on_connect_stream()) {
			let authorities = persistent_data.authority_set.clone();
			let telemetry = telemetry.clone();
			let events = telemetry_on_connect.for_each(move |_| {
				let current_authorities = authorities.current_authorities();
				let set_id = authorities.set_id();
				let maybe_authority_id =
					local_authority_id(&current_authorities, conf.keystore.as_ref());

				let authorities =
					current_authorities.iter().map(|id| id.to_string()).collect::<Vec<_>>();

				let authorities = serde_json::to_string(&authorities).expect(
					"authorities is always at least an empty vector; \
					 elements are always of type string",
				);

				telemetry!(
					telemetry;
					CONSENSUS_INFO;
					"afp.authority_set";
					"authority_id" => maybe_authority_id.map_or("".into(), |s| s.to_string()),
					"authority_set_id" => ?set_id,
					"authorities" => authorities,
				);

				future::ready(())
			});
			future::Either::Left(events)
		} else {
			future::Either::Right(future::pending())
		};

	let voter_work = VoterWork::new(
		client,
		config,
		network,
		select_chain,
		// voting_rule,
		persistent_data,
		voter_commands_rx,
		prometheus_registry,
		shared_voter_state,
		justification_sender,
		telemetry,
	);

	let voter_work = voter_work.map(|res| match res {
		Ok(()) => error!(target: "afp",
			"GRANDPA voter future has concluded naturally, this should be unreachable."
		),
		Err(e) => error!(target: "afp", "GRANDPA voter error: {}", e),
	});

	// Make sure that `telemetry_task` doesn't accidentally finish and kill tendermint.
	let telemetry_task = telemetry_task.then(|_| future::pending::<()>());

	Ok(future::select(voter_work, telemetry_task).map(drop))
}

/// A trait that includes all the client functionalities grandpa requires.
/// Ideally this would be a trait alias, we're not there yet.
/// tracking issue <https://github.com/rust-lang/rust/issues/41517>
pub trait ClientForTendermint<Block, BE>:
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

impl<Block, BE, T> ClientForTendermint<Block, BE> for T
where
	Block: BlockT,
	BE: Backend<Block>,
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

/// Configuration for the Tendermint service.
#[derive(Clone)]
pub struct Config {
	/// The expected duration for a message to be gossiped across the network.
	pub gossip_duration: Duration,
	/// Justification generation period (in blocks). GRANDPA will try to generate justifications
	/// at least every justification_period blocks. There are some other events which might cause
	/// justification generation.
	pub justification_period: u32,
	/// Whether the GRANDPA observer protocol is live on the network and thereby
	/// a full-node not running as a validator is running the GRANDPA observer
	/// protocol (we will only issue catch-up requests to authorities when the
	/// observer protocol is enabled).
	pub observer_enabled: bool,
	/// The role of the local node (i.e. authority, full-node or light).
	pub local_role: sc_network::config::Role,
	/// Some local identifier of the voter.
	pub name: Option<String>,
	/// The keystore that manages the keys of this node.
	pub keystore: Option<SyncCryptoStorePtr>,
	/// TelemetryHandle instance.
	pub telemetry: Option<TelemetryHandle>,
	/// Chain specific PBFT protocol name. See [`crate::protocol_standard_name`].
	pub protocol_name: std::borrow::Cow<'static, str>,
}

impl Config {
	fn name(&self) -> &str {
		self.name.as_deref().unwrap_or("<unknown>")
	}
}

/// A new authority set along with the canonical block it changed at.
#[derive(Debug)]
pub(crate) struct NewAuthoritySet<H, N> {
	pub(crate) canon_number: N,
	pub(crate) canon_hash: H,
	pub(crate) set_id: SetId,
	pub(crate) authorities: AuthorityList,
}

/// Commands issued to the voter.
#[derive(Debug)]
pub(crate) enum VoterCommand<H, N> {
	/// Pause the voter for given reason.
	Pause(String),
	/// New authorities.
	ChangeAuthorities(NewAuthoritySet<H, N>),
}

impl<H, N> fmt::Display for VoterCommand<H, N> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			VoterCommand::Pause(ref reason) => write!(f, "Pausing voter: {}", reason),
			VoterCommand::ChangeAuthorities(_) => write!(f, "Changing authorities"),
		}
	}
}

/// Signals either an early exit of a voter or an error.
#[derive(Debug)]
pub(crate) enum CommandOrError<H, N> {
	/// An error occurred.
	Error(Error),
	/// A command to the voter.
	VoterCommand(VoterCommand<H, N>),
}

impl<H, N> From<Error> for CommandOrError<H, N> {
	fn from(e: Error) -> Self {
		CommandOrError::Error(e)
	}
}

impl<H, N> From<ClientError> for CommandOrError<H, N> {
	fn from(e: ClientError) -> Self {
		CommandOrError::Error(Error::Client(e))
	}
}

impl<H, N> From<finality_tendermint::Error> for CommandOrError<H, N> {
	fn from(e: finality_tendermint::Error) -> Self {
		CommandOrError::Error(Error::from(e))
	}
}

impl<H, N> From<VoterCommand<H, N>> for CommandOrError<H, N> {
	fn from(e: VoterCommand<H, N>) -> Self {
		CommandOrError::VoterCommand(e)
	}
}

impl<H: fmt::Debug, N: fmt::Debug> ::std::error::Error for CommandOrError<H, N> {}

impl<H, N> fmt::Display for CommandOrError<H, N> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			CommandOrError::Error(ref e) => write!(f, "{}", e),
			CommandOrError::VoterCommand(ref cmd) => write!(f, "{}", cmd),
		}
	}
}

/// Errors that can occur while voting in PBFT.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// An error within tendermint.
	#[error("tendermint error: {0}")]
	Tendermint(TendermintError),

	/// A network error.
	#[error("network error: {0}")]
	Network(String),

	/// A blockchain error.
	#[error("blockchain error: {0}")]
	Blockchain(String),

	/// Could not complete a round on disk.
	#[error("could not complete a round on disk: {0}")]
	Client(ClientError),

	/// Could not sign outgoing message
	#[error("could not sign outgoing message: {0}")]
	Signing(String),

	/// An invariant has been violated (e.g. not finalizing pending change blocks in-order)
	#[error("safety invariant has been violated: {0}")]
	Safety(String),

	/// A timer failed to fire.
	#[error("a timer failed to fire: {0}")]
	Timer(std::io::Error),

	/// A runtime api request failed.
	#[error("runtime API request failed: {0}")]
	RuntimeApi(sp_api::ApiError),
}

impl From<TendermintError> for Error {
	fn from(e: TendermintError) -> Self {
		Error::Tendermint(e)
	}
}

impl From<ClientError> for Error {
	fn from(e: ClientError) -> Self {
		Error::Client(e)
	}
}

/// Something which can determine if a block is known.
pub(crate) trait BlockStatus<Block: BlockT> {
	/// Return `Ok(Some(number))` or `Ok(None)` depending on whether the block
	/// is definitely known and has been imported.
	/// If an unexpected error occurs, return that.
	fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error>;
}

impl<Block: BlockT, Client> BlockStatus<Block> for Arc<Client>
where
	Client: HeaderBackend<Block>,
	NumberFor<Block>: BlockNumberOps,
{
	fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error> {
		self.block_number_from_id(&BlockId::Hash(hash))
			.map_err(|e| Error::Blockchain(e.to_string()))
	}
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

impl<Block, Network> BlockSyncRequester<Block> for NetworkBridge<Block, Network>
where
	Block: BlockT,
	Network: NetworkT<Block>,
{
	fn set_sync_fork_request(
		&self,
		peers: Vec<sc_network::PeerId>,
		hash: Block::Hash,
		number: NumberFor<Block>,
	) {
		NetworkBridge::set_sync_fork_request(self, peers, hash, number)
	}
}

/// Checks if this node has any available keys in the keystore for any authority id in the given
/// voter set.  Returns the authority id for which keys are available, or `None` if no keys are
/// available.
fn local_authority_id(
	voters: &VoterSet<AuthorityId>,
	keystore: Option<&SyncCryptoStorePtr>,
) -> Option<AuthorityId> {
	keystore.and_then(|keystore| {
		voters
			.iter()
			.find(|p| SyncCryptoStore::has_keys(&**keystore, &[(p.to_raw_vec(), AuthorityId::ID)]))
			.map(|p| p.clone())
	})
}
