use std::{collections::HashSet, marker::PhantomData};

use parity_scale_codec::{Decode, Encode};
use sp_core::Pair;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};

use crate::primitives::{HotstuffError, HotstuffError::*, ViewNumber};
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthorityPair, AuthoritySignature};

/// Quorum certificate for a block.
#[derive(Debug, Default, Encode, Decode)]
pub struct QC<Block: BlockT> {
	/// Block header hash.
	pub hash: Block::Hash,
	pub view: ViewNumber,
	/// Public key signature pairs for the digest of QC.
	pub votes: Vec<(AuthorityId, AuthoritySignature)>,
}

impl<Block: BlockT> QC<Block> {
	pub fn digest(&self) -> Block::Hash {
		let data = self.hash.encode().append(&mut self.view.encode());
		<<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data)
	}

	// Verify if the number of votes in the QC has exceeded (2/3 + 1) of the total authorities.
	// We are currently not considering the weight of authorities.
	pub fn verify(&self, authorities: AuthorityList) -> Result<(), HotstuffError> {
		let mut used = HashSet::<AuthorityId>::new();
		let mut grant_votes = 0;

		for (authority_id, _) in self.votes.iter() {
			if used.contains(authority_id) {
				return Err(AuthorityReuse(authority_id.clone()))
			}
			used.insert(authority_id.clone());
			grant_votes += 1;
		}

		if grant_votes <= (authorities.len() * 2 / 3) {
			return Err(QCRequiresQuorum)
		}

		let digest = self.digest();
		// TODO parallel verify signature ?
		for (authority_id, signature) in self.votes.iter() {
			if !AuthorityPair::verify(signature, digest, authority_id) {
				return Err(InvalidSignature(authority_id.clone()))
			}
		}

		self.votes.iter().try_for_each(|(authority_id, signature)| {
			if !AuthorityPair::verify(signature, digest, authority_id) {
				return Err(InvalidSignature(authority_id.clone()))
			}
			Ok(())
		})
	}
}

// HotstuffBlock
#[derive(Debug, Encode, Decode)]
pub struct HotstuffBlock<Block: BlockT> {
	pub hash: Block::Hash,
	pub qc: QC<Block>,
	pub view: ViewNumber,
	pub author: AuthorityId,
	pub signature: AuthoritySignature,
}

#[derive(Debug, Encode, Decode)]
pub struct Vote<Block: BlockT> {
	pub hash: Block::Hash,
	pub view: ViewNumber,
	pub voter: AuthorityId,
	pub signature: AuthoritySignature,
}

#[derive(Debug, Encode, Decode)]
pub struct Timeout {}

#[derive(Debug, Encode, Decode)]
pub struct TC {}

#[derive(Debug, Encode, Decode)]
pub enum ConsensusMessage<Block: BlockT> {
	Propose(HotstuffBlock<Block>),
	Vote(Vote<Block>),
	Timeout(Timeout),
	TC(TC),
	SyncRequest(Block::Hash, AuthorityId),
	Phantom(PhantomData<Block>),
}
