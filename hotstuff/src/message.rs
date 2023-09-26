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

	// Add votes to QC.
	// TODO: Need check signature ?
	pub fn add_votes(&mut self, authority_id: AuthorityId, signature: AuthoritySignature) {
		self.votes.push((authority_id, signature));
	}

	// Verify if the number of votes in the QC has exceeded (2/3 + 1) of the total authorities.
	// We are currently not considering the weight of authorities. So a valid QC contains at least 4 votes.
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

		if grant_votes < 4 || grant_votes <= (authorities.len() * 2 / 3) {
			return Err(QCRequiresQuorum)
		}

		let digest = self.digest();
		// TODO parallel verify signature ?
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

//
#[cfg(test)]
mod tests {
	use super::*;

	use sc_keystore::LocalKeystore;
	use sp_consensus_hotstuff::{
		AuthorityId, AuthorityList, AuthorityPair, AuthoritySignature, HOTSTUFF_KEY_TYPE,
	};
	use sp_keystore::KeystorePtr;
	use sp_runtime::testing::{Header as TestHeader, TestXt};

	type TestExtrinsic = TestXt<(), ()>;
	type TestBlock = sp_runtime::testing::Block<TestExtrinsic>;

	fn generate_ed25519_authorities(num: usize, keystore: &KeystorePtr) -> Vec<AuthorityId> {
		let mut authorities = Vec::new();
		for i in 0..num {
			let authority_id = keystore
				.ed25519_generate_new(HOTSTUFF_KEY_TYPE, Some(format!("//User{}", i).as_str()))
				.expect("Creates authority pair")
				.into();
			authorities.push(authority_id);
		}

		authorities
	}


	// TODO some uint tests has same structure. so make table uint tests?
	#[test]
	fn test_qc_verify_with_quorum_should_work() {
		let keystore_path = tempfile::tempdir().expect("Creates keystore path");
		let keystore: KeystorePtr = LocalKeystore::open(keystore_path.path(), None)
			.expect("Creates keystore")
			.into();

		let authorities = generate_ed25519_authorities(4, &keystore);
		let mut wight_authorities = AuthorityList::new();

		let test_block =
			TestBlock { header: TestHeader::new_from_number(10), extrinsics: Vec::new() };

		let mut qc = QC::<TestBlock> { hash: test_block.hash(), view: 10, votes: Vec::new() };
		let digest = qc.digest();

		for authority_id in authorities.iter() {
			wight_authorities.push((authority_id.to_owned(), 10));

			let signature = keystore
				.ed25519_sign(HOTSTUFF_KEY_TYPE, authority_id.as_ref(), &digest.to_fixed_bytes())
				.unwrap()
				.unwrap();
			qc.add_votes(authority_id.to_owned(), signature.into())
		}

		assert!(qc.verify(wight_authorities).is_ok());
	}

	#[test]
	fn test_qc_verify_with_insufficient_votes_should_not_work(){
		let keystore_path = tempfile::tempdir().expect("Creates keystore path");
		let keystore: KeystorePtr = LocalKeystore::open(keystore_path.path(), None)
			.expect("Creates keystore")
			.into();

		let authorities = generate_ed25519_authorities(4, &keystore);
		let mut wight_authorities = AuthorityList::new();

		let test_block =
			TestBlock { header: TestHeader::new_from_number(3), extrinsics: Vec::new() };

		let mut qc = QC::<TestBlock> { hash: test_block.hash(), view: 10, votes: Vec::new() };
		let digest = qc.digest();

		for authority_id in authorities.iter() {
			wight_authorities.push((authority_id.to_owned(), 10));

			let signature = keystore
				.ed25519_sign(HOTSTUFF_KEY_TYPE, authority_id.as_ref(), &digest.to_fixed_bytes())
				.unwrap()
				.unwrap();
			qc.add_votes(authority_id.to_owned(), signature.into())
		}

		assert_eq!(qc.verify(wight_authorities), Err(HotstuffError::QCRequiresQuorum));
	}

	#[test]
	fn test_qc_verify_with_repeated_votes_should_not_work(){}

}
