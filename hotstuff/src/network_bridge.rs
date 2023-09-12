use std::{marker::PhantomData, sync::Arc};

use sc_network::{NetworkBlock, NetworkSyncForkRequest, SyncEventStream, PeerId, ObservedRole};
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, MessageIntent, ValidatorContext};
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver};
use sp_runtime::traits::{Block as BlockT, NumberFor};

use parking_lot::Mutex;

use crate::import::PeerReport;

/// A handle to the network.
///
/// Something that provides the capabilities needed for the `gossip_network::Network` trait.
pub trait Network<Block: BlockT>: GossipNetwork<Block> + Clone + Send + 'static {}

impl<Block, T> Network<Block> for T
where
	Block: BlockT,
	T: GossipNetwork<Block> + Clone + Send + 'static,
{
}

/// A handle to syncing-related services.
///
/// Something that provides the ability to set a fork sync request for a particular block.
pub trait Syncing<Block: BlockT>:
	NetworkSyncForkRequest<Block::Hash, NumberFor<Block>>
	+ NetworkBlock<Block::Hash, NumberFor<Block>>
	+ SyncEventStream
	+ Clone
	+ Send
	+ 'static
{
}

impl<Block, T> Syncing<Block> for T
where
	Block: BlockT,
	T: NetworkSyncForkRequest<Block::Hash, NumberFor<Block>>
		+ NetworkBlock<Block::Hash, NumberFor<Block>>
		+ SyncEventStream
		+ Clone
		+ Send
		+ 'static,
{
}

pub(super) struct GossipValidator<Block: BlockT> {
	_phantom: Option<PhantomData<Block>>,
}

impl<Block: BlockT> GossipValidator<Block> {
	/// Create a new gossip-validator. The current set is initialized to 0. If
	/// `catch_up_enabled` is set to false then the validator will not issue any
	/// catch up requests (useful e.g. when running just the hotstuff observer).
	pub(super) fn new() -> (GossipValidator<Block>, TracingUnboundedReceiver<PeerReport>) {
		let (tx, rx) = tracing_unbounded("mpsc_hotstuff_gossip_validator", 100_000);
		let val = GossipValidator { _phantom: None };

		(val, rx)
	}
}

impl<B: BlockT> sc_network_gossip::Validator<B> for GossipValidator<B> {
	/// New peer is connected.
	fn new_peer(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId, _role: ObservedRole) {
	}

	/// New connection is dropped.
	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId) {}

	/// Validate consensus message.
	fn validate(
		&self,
		context: &mut dyn ValidatorContext<B>,
		sender: &PeerId,
		data: &[u8],
	) -> ValidationResult<B::Hash> {
        // TODO
    }

	/// Produce a closure for validating messages on a given topic.
	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(B::Hash, &[u8]) -> bool + 'a> {
		Box::new(move |_topic, _data| false)
	}

	/// Produce a closure for filtering egress messages.
	fn message_allowed<'a>(
		&'a self,
	) -> Box<dyn FnMut(&PeerId, MessageIntent, &B::Hash, &[u8]) -> bool + 'a> {
		Box::new(move |_who, _intent, _topic, _data| true)
	}
}

/// Bridge between the underlying network service, gossiping hotstuff consensus messages
pub(crate) struct NetworkBridge<B: BlockT, N: Network<B>, S: Syncing<B>> {
	service: N,
	sync: S,
	gossip_engine: Arc<Mutex<GossipEngine<B>>>,
}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> Unpin for NetworkBridge<B, N, S> {}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> NetworkBridge<B, N, S> {
	/// Create a new NetworkBridge to the given NetworkService. Returns the service
	/// handle.
	/// On creation it will register previous rounds' votes with the gossip
	/// service taken from the VoterSetState.
	pub(crate) fn new(service: N, sync: S) -> Self {
		// let protocol = config.protocol_name.clone();

		let protocol = "hotstuff/test";

		let (validator, report_stream) = GossipValidator::new();

		let validator = Arc::new(validator);
		let gossip_engine = Arc::new(Mutex::new(GossipEngine::new(
			service.clone(),
			sync.clone(),
			protocol,
			validator.clone(),
			None,
		)));

		NetworkBridge { service, sync, gossip_engine }
	}
}
