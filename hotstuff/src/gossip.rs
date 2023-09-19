use parity_scale_codec::{Encode, Decode};
use sp_runtime::traits::Block as BlockT;



/// Network level Consensus message with topic information.
#[derive(Debug, Encode, Decode)]
pub(super) struct ConsensusMessage<Block: BlockT> {
    pub(super) topic: Block::Hash,
	/// The round this message is from.
	pub(super) round: u64,
	/// The voter set ID this message is from.
	pub(super) set_id: u64,

	pub hash: Vec<u8>,
}


// // GossipMessage will be replace hotstuff protocol message
// #[derive(Encode, Decode, Debug)]
// pub struct GossipMessage<Block: BlockT>{
//     pub topic: Block::Hash,
//     pub hash: Vec<u8>,
//     pub id: u64,
// }

/// Network level vote message with topic information.
#[derive(Debug, Encode, Decode)]
pub(super) struct VoteMessage<Block: BlockT> {
    pub(super) topic: Block::Hash,
	/// The round this message is from.
	pub(super) round: u64,
	/// The voter set ID this message is from.
	pub(super) set_id: u64,
	// The message itself.
	// pub(super) message: SignedMessage<Block::Header>,
}


/// Hotstuff gossip message type.
/// This is the root type that gets encoded and sent on the network.
#[derive(Debug, Encode, Decode)]
pub(super) enum GossipMessage<Block: BlockT> {
    Consensus(ConsensusMessage<Block>),
	/// Hotstuff message with round and set info.
	Vote(VoteMessage<Block>),
	// Hotstuff commit message with round and set info.
	// Commit(FullCommitMessage<Block>),
	// /// A neighbor packet. Not repropagated.
	// Neighbor(VersionedNeighborPacket<NumberFor<Block>>),
	// /// Hotstuff catch up request message with round and set info. Not repropagated.
	// CatchUpRequest(CatchUpRequestMessage),
	// /// Hotstuff catch up message with round and set info. Not repropagated.
	// CatchUp(FullCatchUpMessage<Block>),
}
