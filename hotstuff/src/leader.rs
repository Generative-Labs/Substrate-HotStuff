
use sp_core::crypto::Pair;

use super::message::{VoteMessage, ConsensusMessage};

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
