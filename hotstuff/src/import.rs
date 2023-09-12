use std::sync::Arc;
use sc_network_gossip::{ValidatorContext, MessageIntent, ValidationResult};
use sc_utils::{notification::NotificationSender, mpsc::{TracingUnboundedReceiver, tracing_unbounded}};
use sp_core::H256;
use sp_runtime::{
	traits::{Block as BlockT, Header as HeaderT, NumberFor, Zero},
	EncodedJustification,
};
use parking_lot::{Mutex, RwLock};
use sc_network::{PeerId, ReputationChange, ObservedRole};
use sp_runtime::traits::Block;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;

/// A block-import handler for Hotstuff.
///
/// This scans each imported block for Hotstuff justifications and verifies them.
/// Wraps a `inner: BlockImport` and ultimately defers to it.
///
/// When using Hotstuff, the block import worker should be using this block import object.
pub struct HotstuffBlockImport<Block: BlockT, Backend, RuntimeApi, I> {
	backend: Arc<Backend>,
	runtime: Arc<RuntimeApi>,
	inner: I,
	_phantom: PhantomData<Block>,
}


struct PeerData<B: Block> {
	last_voted_on: NumberFor<B>,
}


/// Report specifying a reputation change for a given peer.
#[derive(Debug, PartialEq)]
pub(crate) struct PeerReport {
	pub who: PeerId,
	pub cost_benefit: ReputationChange,
}

/// Keep a simple map of connected peers
/// and the most recent voting round they participated in.
pub struct KnownPeers<B: Block> {
	live: HashMap<PeerId, PeerData<B>>,
}

impl<B: Block> KnownPeers<B> {
	pub fn new() -> Self {
		Self { live: HashMap::new() }
	}
}

pub(crate) struct GossipValidator<B>
where
	B: Block,
{

	known_peers: Arc<Mutex<KnownPeers<B>>>,
}


impl<B> GossipValidator<B>
where
	B: Block,
{
	pub(crate) fn new(
		known_peers: Arc<Mutex<KnownPeers<B>>>,
	) -> (GossipValidator<B>, TracingUnboundedReceiver<PeerReport>) {
		let (tx, rx) = tracing_unbounded("mpsc_hotstuff_gossip_validator", 10_000);
		let val = GossipValidator {
			known_peers,
		};
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
		// topic
		sc_network_gossip::ValidationResult::Discard
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
