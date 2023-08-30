
use sp_core::crypto::Pair;

use super::message::{VoteMessage, ConsensusMessage};

// hotstuff validator trait
pub trait HotstuffValidator<P: Pair> {
    fn send_vote(&self, vote: &VoteMessage<P>);

    // As a validator process consensus message gossip from leader
    fn process_consensus_message(&self, consensus_msg: &ConsensusMessage);

    // As a validator; Validate  consensus message gossip from leader
    fn validate_consensus_message(&self, consensus_msg: &ConsensusMessage) -> bool;
}

