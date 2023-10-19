use sc_client_api::backend::Backend;

use sp_runtime::traits::Block as BlockT;

use crate::{
	authorities::{AuthoritySet, DelayKind, PendingChange, SharedAuthoritySet},
	environment,
	justification::GrandpaJustification,
	notification::GrandpaJustificationSender,
	AuthoritySetChanges, ClientForGrandpa, CommandOrError, Error, NewAuthoritySet, VoterCommand,
	LOG_TARGET,
};

/// A block-import handler for HOTSTUFF.
///
/// This scans each imported block for signals of changing authority set.
/// If the block being imported enacts an authority set change then:
/// - If the current authority set is still live: we import the block
/// - Otherwise, the block must include a valid justification.
///
/// When using HOTSTUFF, the block import worker should be using this block import
/// object.
pub struct HotstuffBlockImport<Backend, Block: BlockT, Client, SC> {
	inner: Arc<Client>,
	justification_import_period: u32,
	select_chain: SC,
	authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
	send_voter_commands: TracingUnboundedSender<VoterCommand<Block::Hash, NumberFor<Block>>>,
	authority_set_hard_forks: HashMap<Block::Hash, PendingChange<Block::Hash, NumberFor<Block>>>,
	justification_sender: HotstuffJustificationSender<Block>,
	telemetry: Option<TelemetryHandle>,
	_phantom: PhantomData<Backend>,
}
