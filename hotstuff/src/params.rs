use sc_telemetry::TelemetryHandle;
use sp_keystore::KeystorePtr;
use std::sync::Arc;

/// Run `hotstuff` in a compatibility mode.
///
/// This is required for when the chain was launched and later there
/// was a consensus breaking change.
#[derive(Debug, Clone)]
pub enum CompatibilityMode<N> {
	/// Don't use any compatibility mode.
	None,
	/// Call `initialize_block` before doing any runtime calls.
	///
	/// Previously the node would execute `initialize_block` before fetchting the authorities
	/// from the runtime. This behaviour changed in: <https://github.com/paritytech/substrate/pull/9132>
	///
	/// By calling `initialize_block` before fetching the authorities, on a block that
	/// would enact a new validator set, the block would already be build/sealed by an
	/// authority of the new set. With this mode disabled (the default) a block that enacts a new
	/// set isn't sealed/built by an authority of the new set, however to make new nodes be able to
	/// sync old chains this compatibility mode exists.
	UseInitializeBlock {
		/// The block number until this compatibility mode should be executed. The first runtime
		/// call in the context of the `until` block (importing it/building it) will disable the
		/// compatibility mode (i.e. at `until` the default rules will apply). When enabling this
		/// compatibility mode the `until` block should be a future block on which all nodes will
		/// have upgraded to a release that includes the updated compatibility mode configuration.
		/// At `until` block there will be a hard fork when the authority set changes, between the
		/// old nodes (running with `initialize_block`, i.e. without the compatibility mode
		/// configuration) and the new nodes.
		until: N,
	},
}

/// Parameters of [`start_hotstuff`].
pub struct StartHotstuffParams<C, SC, I, PF, SO, L, N> {
	/// The client to interact with the chain.
	pub client: Arc<C>,
	/// A select chain implementation to select the best block.
	pub select_chain: SC,
	/// The block import.
	pub block_import: I,
	/// The proposer factory to build proposer instances.
	pub proposer_factory: PF,
	/// The sync oracle that can give us the current sync status.
	pub sync_oracle: SO,
	/// Hook into the sync module to control the justification sync process.
	pub justification_sync_link: L,

	/// Should we force the authoring of blocks?
	pub force_authoring: bool,
	/// The keystore used by the node.
	pub keystore: KeystorePtr,

	/// Telemetry instance used to report telemetry metrics.
	pub telemetry: Option<TelemetryHandle>,

	// If in doubt, use `Default::default()`.
	pub compatibility_mode: CompatibilityMode<N>,
}

impl<N> Default for CompatibilityMode<N> {
	fn default() -> Self {
		Self::None
	}
}
