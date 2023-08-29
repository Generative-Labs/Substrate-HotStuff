pub struct HotstuffVotingSystem {
    node: Box<dyn HotstuffVoteWork>,
}

impl HotstuffVotingSystem {
    pub fn new(node: Box<dyn HotstuffVoteWork>) -> Self {
        Self { node }
    }

    pub fn process_consensus_message(&self, consensus_msg: &ConsensusMessage) {
        if self.node.validate_consensus_message(consensus_msg) {
            self.node.process_consensus_message(consensus_msg);

            if let Some(leader) = self.node.as_leader() {
                leader.prepare_message(consensus_msg);
                leader.precommit_message(consensus_msg);
                leader.commit_message(consensus_msg);
                leader.decide_message(consensus_msg);
            } else if let Some(validator) = self.node.as_validator() {
                validator.send_vote(&VoteInfo { /* vote details */ });
            }
        } else {
            // Handle invalid consensus message
            unimplemented!();
        }
    }

    // Add more methods as needed for voting-related executions
}