use crypto;
use sp_core::{crypto::Pair, H256};

pub type AuthorityId<P> = <P as Pair>::Public;

pub struct VoteMessage<P: Pair> {
	pub block_hash: H256,
	pub round_number: u64,
	pub voter: AuthorityId<P>,
	pub signature: crypto::Signature,
}

pub struct ConsensusMessage {
	pub round_number: u64,
	pub parent_block_hash: H256,
	pub block_hash: H256,
	// pub proposal: Proposer::Proposal,
}
