use std::marker::PhantomData;

use sp_core::crypto::Pair;

use crate::{leader, message, validator};

pub trait HotstuffVoteWork<P: Pair>:
	leader::HotstuffLeader<P> + validator::HotstuffValidator<P>
{
}

pub struct HotstuffVoter<P: Pair> {
	pub _phantom_data: PhantomData<P>,
}

impl<P: Pair> leader::HotstuffLeader<P> for HotstuffVoter<P> {
	fn broadcast_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	fn prepare_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	fn precommit_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	fn commit_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	fn decide_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	fn vote_handler(&self, _vote: &message::VoteMessage<P>) {
		// leader handler voting from validators
	}

	fn validate_vote(&self, _vote: &message::VoteMessage<P>) {
		// leader validate vote message
		_vote.block_hash;
	}
}

/// Hotstuff validator trait implementation
impl<P: Pair> validator::HotstuffValidator<P> for HotstuffVoter<P> {
	fn send_vote(&self, _vote: &message::VoteMessage<P>) {
		unimplemented!();
	}

	// As a validator process consensus message gossip from leader
	fn process_consensus_message(&self, _consensus_msg: &message::ConsensusMessage) {
		unimplemented!();
	}

	// As a validator; Validate  consensus message gossip from leader
	fn validate_consensus_message(&self, _consensus_msg: &message::ConsensusMessage) -> bool {
		true
	}
}
