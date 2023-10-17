use std::{collections::HashSet, fmt, marker::PhantomData};

use parity_scale_codec::{Decode, Encode};
use sp_core::Pair;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};

use crate::primitives::{HotstuffError, HotstuffError::*, ViewNumber};
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthorityPair, AuthoritySignature};

#[cfg(test)]
#[path = "tests/message_tests.rs"]
pub mod message_tests;

/// Quorum certificate for a block.
#[derive(Debug, Eq, Clone, Encode, Decode)]
pub struct QC<Block: BlockT> {
	/// Hotstuff proposal hash.
	pub proposal_hash: Block::Hash,
	pub view: ViewNumber,
	/// Public key signature pairs for the digest of QC.
	pub votes: Vec<(AuthorityId, AuthoritySignature)>,
}

impl<B: BlockT> Default for QC<B> {
	fn default() -> Self {
		Self {
			proposal_hash: Default::default(),
			view: Default::default(),
			votes: Default::default(),
		}
	}
}

impl<Block: BlockT> QC<Block> {
	pub fn digest(&self) -> Block::Hash {
		let mut data = self.proposal_hash.encode();
		data.append(&mut self.view.encode());

		<<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data)
	}

	// Add votes to QC.
	// TODO: Need check signature ?
	pub fn add_votes(&mut self, authority_id: AuthorityId, signature: AuthoritySignature) {
		self.votes.push((authority_id, signature));
	}

	// Verify if the number of votes in the QC has exceeded (2/3 + 1) of the total authorities.
	// We are currently not considering the weight of authorities.
	pub fn verify(&self, authorities: &AuthorityList) -> Result<(), HotstuffError> {
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
			return Err(InsufficientQuorum)
		}

		let digest = self.digest();

		for (voter, signature) in self.votes.iter() {
			if !AuthorityPair::verify(signature, digest, voter) {
				return Err(InvalidSignature(voter.clone()))
			}
		}
		Ok(())
	}
}

impl<Block: BlockT> PartialEq for QC<Block> {
	fn eq(&self, other: &Self) -> bool {
		self.proposal_hash == other.proposal_hash && self.view == other.view
	}
}

#[derive(Debug, Copy, Clone, Encode, Decode)]
pub struct Payload<Block: BlockT> {
	pub block_hash: Block::Hash,
	pub block_number: <Block::Header as HeaderT>::Number,
}

impl<Block: BlockT> fmt::Display for Payload<Block> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{{ block_hash: {} ", self.block_hash)?;
		write!(f, "  block_number: {} }}", self.block_number)
	}
}

// Hotstuff Proposal
#[derive(Debug, Clone, Encode, Decode)]
pub struct Proposal<Block: BlockT> {
	// QC of parent proposal.
	pub qc: QC<Block>,
	pub tc: Option<TC<Block>>,
	// payload represents the block hash in the blockchain that will be finalized.
	pub payload: Payload<Block>,
	pub view: ViewNumber,
	// The authority id of current hotstuff block author,
	// which is not the origin block producer.
	pub author: AuthorityId,
	// Signature of current block digest.
	pub signature: Option<AuthoritySignature>,
}

impl<Block: BlockT> Proposal<Block> {
	pub fn new(
		qc: QC<Block>,
		tc: Option<TC<Block>>,
		payload: Payload<Block>,
		view: ViewNumber,
		author: AuthorityId,
		signature: Option<AuthoritySignature>,
	) -> Self {
		Proposal { qc, tc, payload, view, author, signature }
	}

	pub fn parent_hash(&self) -> Block::Hash {
		self.qc.proposal_hash
	}

	pub fn digest(&self) -> Block::Hash {
		let mut data = self.author.encode();
		data.append(&mut self.payload.encode());
		data.append(&mut self.view.encode());
		data.append(&mut self.qc.proposal_hash.encode());

		<<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data)
	}

	pub fn verify(&self, authorities: &AuthorityList) -> Result<(), HotstuffError> {
		authorities
			.iter()
			.find(|authority| authority.0 == self.author)
			.ok_or(HotstuffError::UnknownAuthority(self.author.to_owned()))?;

		self.signature.as_ref().ok_or(NullSignature).and_then(|signature| {
			if !AuthorityPair::verify(signature, self.digest(), &self.author) {
				return Err(InvalidSignature(self.author.to_owned()))
			}
			Ok(())
		})?;

		if self.qc != QC::<Block>::default() {
			self.qc.verify(authorities)?;
		}

		Ok(())
	}
}

// Vote for a Proposal
#[derive(Debug, Clone, Encode, Decode)]
pub struct Vote<Block: BlockT> {
	pub proposal_hash: Block::Hash,
	pub view: ViewNumber,
	pub voter: AuthorityId,
	pub signature: Option<AuthoritySignature>,
}

impl<Block: BlockT> Vote<Block> {
	pub fn new(proposal_hash: Block::Hash, proposal_view: ViewNumber, voter: AuthorityId) -> Self {
		Self { proposal_hash, view: proposal_view, voter, signature: None }
	}

	pub fn digest(&self) -> Block::Hash {
		let mut data = self.proposal_hash.encode();
		data.append(&mut self.view.encode());

		<<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data)
	}

	pub fn verify(&self, authorities: &AuthorityList) -> Result<(), HotstuffError> {
		authorities
			.iter()
			.find(|authority| authority.0 == self.voter)
			.ok_or(HotstuffError::UnknownAuthority(self.voter.to_owned()))?;

		self.signature.as_ref().ok_or(NullSignature).and_then(|signature| {
			if !AuthorityPair::verify(signature, self.digest(), &self.voter) {
				return Err(InvalidSignature(self.voter.to_owned()))
			}
			Ok(())
		})
	}
}

// Timeout notification
#[derive(Debug, Clone, Encode, Decode)]
pub struct Timeout<Block: BlockT> {
	// The hightest QC of local node.
	pub high_qc: QC<Block>,
	// The view of local node at timeout.
	pub view: ViewNumber,
	pub voter: AuthorityId,
	pub signature: Option<AuthoritySignature>,
}

impl<Block: BlockT> Timeout<Block> {
	pub fn digest(&self) -> Block::Hash {
		let mut data = self.view.encode();
		data.append(&mut self.high_qc.view.encode());

		<<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data)
	}

	pub fn verify(&self, authorities: &AuthorityList) -> Result<(), HotstuffError> {
		authorities
			.iter()
			.find(|authority| authority.0 == self.voter)
			.ok_or(HotstuffError::UnknownAuthority(self.voter.to_owned()))?;

		self.signature.as_ref().ok_or(NullSignature).and_then(|signature| {
			if !AuthorityPair::verify(signature, self.digest(), &self.voter) {
				return Err(InvalidSignature(self.voter.to_owned()))
			}
			Ok(())
		})?;

		if self.high_qc != QC::<Block>::default() {
			self.high_qc.verify(authorities)?;
		}
		Ok(())
	}
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct TC<Block: BlockT> {
	pub view: ViewNumber,
	pub votes: Vec<(AuthorityId, AuthoritySignature, ViewNumber)>,
	pub _phantom: PhantomData<Block>,
}

impl<Block: BlockT> TC<Block> {
	pub fn verify(&self, authorities: &AuthorityList) -> Result<(), HotstuffError> {
		let mut used = HashSet::<AuthorityId>::new();
		let mut grant_votes = 0;

		for (authority_id, _, _) in self.votes.iter() {
			if used.contains(authority_id) {
				return Err(AuthorityReuse(authority_id.clone()))
			}
			used.insert(authority_id.clone());
			grant_votes += 1;
		}

		if grant_votes <= (authorities.len() * 2 / 3) {
			return Err(InsufficientQuorum)
		}

		for (voter, signature, view) in self.votes.iter() {
			// TODO a better way to construct `Timeout`, then call `Timeout::digest()`
			let mut data = self.view.encode();
			data.append(&mut view.encode());
			let digest = <<Block::Header as HeaderT>::Hashing as HashT>::hash_of(&data);

			if !AuthorityPair::verify(signature, digest, voter) {
				return Err(InvalidSignature(voter.clone()))
			}
		}

		Ok(())
	}
}

#[derive(Debug, Encode, Decode)]
pub enum ConsensusMessage<B: BlockT> {
	Propose(Proposal<B>),
	Vote(Vote<B>),
	Timeout(Timeout<B>),
	TC(TC<B>),
	SyncRequest(B::Hash, AuthorityId),
	Phantom(PhantomData<B>),
}

impl<Block: BlockT> ConsensusMessage<Block> {
	pub fn gossip_topic() -> Block::Hash {
		// TODO maybe use Lazy then just call hash once.
		<<Block::Header as HeaderT>::Hashing as HashT>::hash(b"hotstuff/consensus")
	}
}
