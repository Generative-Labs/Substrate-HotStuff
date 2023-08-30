
use sp_core::H256;
// use sp_consensus::Proposer;
use std::marker::PhantomData;


/// Identity of a hotstuff authority.
// pub type AuthorityId = app::Public;
use crypto;
use sp_core::crypto::Pair;
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

// hotstuff leader trait
pub trait HotstuffLeader<P: Pair> {
    fn broadcast_message(&self, consensus_msg: &ConsensusMessage);
    fn prepare_message(&self, consensus_msg: &ConsensusMessage);
    fn precommit_message(&self, consensus_msg: &ConsensusMessage);
    fn commit_message(&self, consensus_msg: &ConsensusMessage);
    fn decide_message(&self, consensus_msg: &ConsensusMessage);
    fn vote_handler(&self, vote: &VoteMessage<P>);   // As a leader handler vote from other validators
    fn validate_vote(&self, vote: &VoteMessage<P>);    // validate vote from validator
}

// hotstuff validator trait
pub trait HotstuffValidator<P: Pair> {
    fn send_vote(&self, vote: &VoteMessage<P>);

    // As a validator process consensus message gossip from leader
    fn process_consensus_message(&self, consensus_msg: &ConsensusMessage);

    // As a validator; Validate  consensus message gossip from leader
    fn validate_consensus_message(&self, consensus_msg: &ConsensusMessage) -> bool;
}

pub trait HotstuffVoteWork<P: Pair>: HotstuffLeader<P> + HotstuffValidator<P> {}

pub struct HotstuffVoter<P: Pair> {

    pub _phantom_data: PhantomData<P>,

}

impl<P: Pair> HotstuffLeader<P> for HotstuffVoter<P> {

    fn broadcast_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    fn prepare_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    fn precommit_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    fn commit_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    fn decide_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    fn vote_handler(&self, _vote: &VoteMessage<P>) {
        // leader handler voting from validators
    }

    fn validate_vote(&self, _vote:&VoteMessage<P>) {
        // leader validate vote message
        _vote.block_hash;
    }
}

/// Hotstuff validator trait implementation
impl<P: Pair> HotstuffValidator<P> for HotstuffVoter<P> {

    fn send_vote(&self, _vote: &VoteMessage<P>) {
        unimplemented!();
    }

    // As a validator process consensus message gossip from leader
    fn process_consensus_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }

    // As a validator; Validate  consensus message gossip from leader
    fn validate_consensus_message(&self, _consensus_msg: &ConsensusMessage) -> bool {
        true
    }
}
