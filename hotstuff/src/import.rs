use std::sync::Arc;
use sc_client_api::Backend;
use sc_network_gossip::{ValidatorContext, MessageIntent, ValidationResult};
use sc_telemetry::log::debug;
use sc_utils::{notification::NotificationSender, mpsc::{TracingUnboundedReceiver, tracing_unbounded}};
use sp_consensus_hotstuff::HOTSTUFF_ENGINE_ID;
use sp_core::H256;
use sp_runtime::{
	traits::{Block as BlockT, Header as HeaderT, NumberFor, Zero},
	EncodedJustification, Justification,
};
use log;
use parking_lot::{Mutex, RwLock};
use sc_network::{PeerId, ReputationChange, ObservedRole};
use sp_runtime::traits::Block;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use sp_api::TransactionFor;
use sp_blockchain::BlockStatus;

use sp_consensus_grandpa::GrandpaApi;

use sc_consensus::{BlockImport, ImportResult, BlockImportParams, BlockCheckParams, JustificationImport};
use sp_consensus::{Error as ConsensusError};

use crate::client::ClientForHotstuff;

const LOG_TARGET: &str  = "hotstuff";

/// A block-import handler for Hotstuff.
///
/// This scans each imported block for Hotstuff justifications and verifies them.
/// Wraps a `inner: BlockImport` and ultimately defers to it.
///
/// When using Hotstuff, the block import worker should be using this block import object.
pub struct HotstuffBlockImport<Backend, Block: BlockT, Client> {
	backend: PhantomData<Backend>,
	inner: Arc<Client>,
	_phantom: PhantomData<Block>,
}


impl<Backend, Block: BlockT, Client> Clone
	for HotstuffBlockImport<Backend, Block, Client>
{
	fn clone(&self) -> Self {
		HotstuffBlockImport {
			inner: self.inner.clone(),
			backend: PhantomData,
			_phantom: PhantomData,
		}
	}
}

impl<Backend, Block: BlockT, Client> HotstuffBlockImport<Backend, Block, Client>
{
	pub fn new(
		inner: Arc<Client>,
	) -> HotstuffBlockImport<Backend, Block, Client> {
		HotstuffBlockImport {
			inner: inner,
			backend: PhantomData,
			_phantom: PhantomData,
		}
	}
}


#[async_trait::async_trait]
impl<BE, Block: BlockT, Client> BlockImport<Block> for HotstuffBlockImport<BE, Block, Client>
where
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	Client::Api: GrandpaApi<Block>,
	for<'a> &'a Client:
		BlockImport<Block, Error = ConsensusError, Transaction = TransactionFor<Client, Block>>,
	TransactionFor<Client, Block>: 'static,
{
	type Error = ConsensusError;
	type Transaction = TransactionFor<Client, Block>;


	async fn check_block(
		&mut self,
		block: BlockCheckParams<Block>,
	) -> Result<ImportResult, Self::Error> {
		self.inner.check_block(block).await
	}

	async fn import_block(
		&mut self,
		mut block: BlockImportParams<Block, Self::Transaction>,
	) -> Result<ImportResult, Self::Error> {
		let hash = block.post_hash();
		let number = *block.header.number();

		println!("🔥 >>> import block: hash: {:?} number:{:?}", hash, number);
		match self.inner.status(hash) {
			Ok(BlockStatus::InChain) => {
				// Strip justifications when re-importing an existing block.
				let _justifications = block.justifications.take();
				return (&*self.inner).import_block(block).await
			},
			Ok(BlockStatus::Unknown) => {},
			Err(e) => return Err(ConsensusError::ClientImport(e.to_string())),
		}
		// if block.with_state() {
		// 	return self.import_state(block).await
		// }

		let import_result = (&*self.inner).import_block(block).await;

		let mut imported_aux = {
			match import_result {
				Ok(ImportResult::Imported(aux)) => aux,
				Ok(r) => {
					return Ok(r)
				},
				Err(e) => {
					return Err(ConsensusError::ClientImport(e.to_string()))
				},
			}
		};

		// TODO
		Ok(ImportResult::Imported(imported_aux))
	}
}


impl<BE, Block: BlockT, Client> HotstuffBlockImport<BE, Block, Client>
where
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
{
	/// Import a block justification and finalize the block.
	///
	/// If `enacts_change` is set to true, then finalizing this block *must*
	/// enact an authority set change, the function will panic otherwise.
	fn import_justification(
		&mut self,
		hash: Block::Hash,
		number: NumberFor<Block>,
		justification: Justification,
		enacts_change: bool,
		initial_sync: bool,
	) -> Result<(), ConsensusError> {
		// NOTE: lock must be held through writing to DB to avoid race. this lock
		//       also implicitly synchronizes the check for last finalized number
		//       below.
		let client = self.inner.clone();
		let status = client.info();

		// if number <= status.finalized_number && client.hash(number)? == Some(hash) {
		// 	// This can happen after a forced change (triggered manually from the runtime when
		// 	// finality is stalled), since the voter will be restarted at the median last finalized
		// 	// block, which can be lower than the local best finalized block.
		// 	log::warn!(target: LOG_TARGET, "Re-finalized block #{:?} ({:?}) in the canonical chain, current best finalized is #{:?}",
		// 			hash,
		// 			number,
		// 			status.finalized_number,
		// 	);
	
		// 	return Ok(())
		// }
	

		// TODO

		// if justification.0 != HOTSTUFF_ENGINE_ID {
		// 	// TODO: the import queue needs to be refactored to be able dispatch to the correct
		// 	// `JustificationImport` instance based on `ConsensusEngineId`, or we need to build a
		// 	// justification import pipeline similar to what we do for `BlockImport`. In the
		// 	// meantime we'll just drop the justification, since this is only used for BEEFY which
		// 	// is still WIP.
		// 	return Ok(())
		// }


		// let result = finalize_block(
		// 	self.inner.clone(),
		// 	&self.authority_set,
		// 	None,
		// 	hash,
		// 	number,
		// 	justification.into(),
		// 	initial_sync,
		// 	Some(&self.justification_sender),
		// 	self.telemetry.clone(),
		// );
		println!("🔥💃🏻 import_justification finalize_block start");
		let _res: Result<(), sp_blockchain::Error> = self.inner.finalize_block(hash, None, true);
		match _res {
			Ok(()) => {
				println!("🔥💃🏻 success finalize_block");
			}
			Err(err) => {
				println!("🔥💃🏻 finalize_block error: {:?}", err);
			}
		}

		Ok(())
	}
}


#[async_trait::async_trait]
impl<BE, Block: BlockT, Client> JustificationImport<Block>
	for HotstuffBlockImport<BE, Block, Client>
where
	NumberFor<Block>: finality_grandpa::BlockNumberOps,
	BE: Backend<Block>,
	Client: ClientForHotstuff<Block, BE>,
{
	type Error = ConsensusError;

	async fn import_justification(
		&mut self,
		hash: Block::Hash,
		number: NumberFor<Block>,
		justification: Justification,
	) -> Result<(), Self::Error> {
		// this justification was requested by the sync service, therefore we
		// are not sure if it should enact a change or not. it could have been a
		// request made as part of initial sync but that means the justification
		// wasn't part of the block and was requested asynchronously, probably
		// makes sense to log in that case.
		HotstuffBlockImport::import_justification(self, hash, number, justification, false, false)
	}

	async fn on_start(&mut self) -> Vec<(Block::Hash, NumberFor<Block>)> {
		let mut out = Vec::new();
		let chain_info = self.inner.info();

		// TODO

		out
	}
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
