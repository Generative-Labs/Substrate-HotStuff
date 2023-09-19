use std::{marker::PhantomData, sync::Arc};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::prelude::*;
use log::info;
use sc_network::{NetworkBlock, NetworkSyncForkRequest, NetworkStateInfo, SyncEventStream, PeerId, ObservedRole, ProtocolName};
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, MessageIntent, ValidatorContext, ValidationResult};
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver};
use sp_consensus_hotstuff::RoundNumber;
use sp_runtime::traits::{Block as BlockT, NumberFor, Header as HeaderT, Hash as HashT};
use sp_core::Decode;

use parking_lot::Mutex;

use crate::gossip;
use crate::import::PeerReport;

/// A handle to the network.
///
/// Something that provides the capabilities needed for the `gossip_network::Network` trait.
pub trait Network<Block: BlockT>: GossipNetwork<Block> + NetworkStateInfo + Clone + Send + 'static {}

impl<Block, T> Network<Block> for T
where
	Block: BlockT,
	T: GossipNetwork<Block> + NetworkStateInfo + Clone + Send + 'static,
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
		let (_tx, rx) = tracing_unbounded("mpsc_hotstuff_gossip_validator", 100_000);
		let val = GossipValidator { _phantom: None };

		(val, rx)
	}

	pub fn do_validate(
		&self,
		mut data: &[u8],
	)->Option<Block::Hash>{
		match gossip::GossipMessage::<Block>::decode(&mut data){
			Ok(gossip::GossipMessage::Vote(res)) => Some(res.topic),
			Ok(gossip::GossipMessage::Consensus(res)) => Some(res.topic),
    		Err(e) => {
				info!("~~ GossipValidator decode data failed {}", e);
				None
			},
		}
	}
}

impl<B: BlockT> sc_network_gossip::Validator<B> for GossipValidator<B> {
	/// New peer is connected.
	fn new_peer(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId, _role: ObservedRole) {
		// println!("~~~~【GossipValidator】:: new_peer PeerId:{}", who);
		// context.send_message(who, Vec::from("hotstuff send back"));
	}

	/// New connection is dropped.
	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId) {
		println!("【GossipValidator】:: peer_disconnected PeerId:{}", _who);
	}

	/// Validate consensus message.
	fn validate(
		&self,
		_context: &mut dyn ValidatorContext<B>,
		_sender: &PeerId,
		data: &[u8],
	) -> ValidationResult<B::Hash> {
		match self.do_validate(data){
    		Some(topic) => ValidationResult::ProcessAndKeep(topic),
    		None => ValidationResult::Discard,
		}
    }

	/// Produce a closure for validating messages on a given topic.
	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(B::Hash, &[u8]) -> bool + 'a> {
		Box::new(move |_topic, _data| false)
	}

	/// Produce a closure for filtering egress messages.
	fn message_allowed<'a>(
		&'a self,
	) -> Box<dyn FnMut(&PeerId, MessageIntent, &B::Hash, &[u8]) -> bool + 'a> {
		Box::new(move |_who, _intent, _topic, _data|{
			// println!("message_allowed who: {}, {:#?}", who, data);
			true
		})
	}
}

/// Bridge between the underlying network service, gossiping hotstuff consensus messages
pub struct HotstuffNetworkBridge<B: BlockT, N: Network<B>, S: Syncing<B>> {
	pub service: N,
	pub sync: S,
	pub gossip_engine: Arc<Mutex<GossipEngine<B>>>,
}

pub type SetId = u64;

/// Create a unique topic for a round and set-id combo.
#[allow(unused)]
pub(crate) fn round_topic<B: BlockT>(round: RoundNumber, set_id: SetId) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-{}", set_id, round).as_bytes())
}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> Unpin for HotstuffNetworkBridge<B, N, S> {}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> HotstuffNetworkBridge<B, N, S> {
	/// Create a new HotstuffNetworkBridge to the given NetworkService. Returns the service
	/// handle.
	/// On creation it will register previous rounds' votes with the gossip
	/// service taken from the VoterSetState.
	pub fn new(
		service: N,
		sync: S,
		protocol_name: ProtocolName,
	) -> Self {
		// let protocol = config.protocol_name.clone();
		println!(">>>HotstuffNetworkBridge start 🔥");

		let (validator, _report_stream) = GossipValidator::new();

		let validator = Arc::new(validator);
		let gossip_engine = Arc::new(Mutex::new(GossipEngine::new(
			service.clone(),
			sync.clone(),
			protocol_name,
			validator.clone(),
			None,
		)));

		//let topic = <<B::Header as HeaderT>::Hashing>::hash(b"topic name");
		// gossip_engine.lock().register_gossip_message(topic, vec![1, 2]);
		// println!(">>>HotstuffNetworkBridge register_gossip_message 🔥");

		HotstuffNetworkBridge { service, sync, gossip_engine }
	}

	pub fn local_peer_id(&self)->PeerId{
		self.service.local_peer_id()
	}
}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> Future for HotstuffNetworkBridge<B, N, S> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		match self.gossip_engine.lock().poll_unpin(cx) {
        	Poll::Ready(()) =>{
				return Poll::Ready(())
			},
        	Poll::Pending => {},
        };
		
		Poll::Pending
    }
}