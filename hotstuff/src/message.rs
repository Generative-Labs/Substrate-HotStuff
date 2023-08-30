

use sp_core::H256;
// use sp_consensus::Proposer;

use sp_core::crypto::Pair;

use crypto;

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
