

pub struct ConsensusMessage {

}

pub struct VoteInfo {

}

// hotstuff leader trait
pub trait HotstuffLeader {
    fn broadcast_message(&self, consensus_msg: &ConsensusMessage);
    fn prepare_message(&self, consensus_msg: &ConsensusMessage);
    fn precommit_message(&self, consensus_msg: &ConsensusMessage);
    fn commit_message(&self, consensus_msg: &ConsensusMessage);
    fn decide_message(&self, consensus_msg: &ConsensusMessage);
    fn vote_handler(&self, vote: &VoteInfo);   // As a leader handler vote from other validators
    fn validate_vote(&self, vote:&VoteInfo);    // validate vote from validator
}

// hotstuff validator trait
pub trait HotstuffValidator {
    fn send_vote(&self, vote: &VoteInfo);

    // As a validator process consensus message gossip from leader
    fn process_consensus_message(&self, consensus_msg: &ConsensusMessage);

    // As a validator; Validate  consensus message gossip from leader
    fn validate_consensus_message(&self, consensus_msg: &ConsensusMessage);
}

pub trait HotstuffVoteWork: HotstuffLeader + HotstuffValidator {}

pub struct HotstuffVoter;

impl HotstuffLeader for HotstuffVoter {
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
    fn vote_handler(&self, _vote: &VoteInfo) {
        // TODO
    }
    fn validate_vote(&self, _vote:&VoteInfo) {
        unimplemented!();
    }
}


impl HotstuffValidator for HotstuffVoter {
    fn send_vote(&self, _vote: &VoteInfo) {
        unimplemented!();
    }

    // As a validator process consensus message gossip from leader
    fn process_consensus_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();

    }

    // As a validator; Validate  consensus message gossip from leader
    fn validate_consensus_message(&self, _consensus_msg: &ConsensusMessage) {
        unimplemented!();
    }
}
