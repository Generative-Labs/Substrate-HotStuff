
// pub mod block_producer;
// pub mod finalize_block;
pub mod leader;
pub mod message;
pub mod params;

pub mod qc;
pub mod types;
pub mod validator;
pub mod voter;
// pub mod voting_handler;
pub mod worker;
mod finalize_block;


#[cfg(test)]
mod tests {
	use crypto::Digest;
	use ed25519_dalek::Digest as _;
	use ed25519_dalek::Sha512;

	use sp_core::H256;

	use std::marker::PhantomData;

	use sp_application_crypto::Pair;
	use sp_application_crypto::ed25519::AppPair;

	use crate::voter::HotstuffVoter;
	use crate::leader::HotstuffLeader;
	use crate::validator::HotstuffValidator;
	use crate::message::{VoteMessage, ConsensusMessage};

	pub trait Hash {
		fn digest(&self) -> Digest;
	}

	impl Hash for &[u8] {
		fn digest(&self) -> Digest {
			Digest(Sha512::digest(self).as_slice()[..32].try_into().unwrap())
		}
	}

    #[test]
    fn test_start_hotstuff() {
        // Arrange
        // let result = start_hotstuff().expect("hotstuff entry function");

        // Assert
        // Add your assertions here if needed
    }

	fn get_keypair() -> (crypto::PublicKey, crypto::SecretKey) {
		// Get a keypair.
		let (public_key, secret_key) = crypto::keys().pop().unwrap();
		(public_key, secret_key)
	}

	#[test]
	fn test_hotstuff_vote_handler() {
		let key_pair = AppPair::from_seed(&[1; 32]);

		let voter_id = key_pair.public();

		// Get a keypair.
		let (_public_key, secret_key) = get_keypair();

		// Make signature.
		let message: &[u8] = b"Hello, world!";
		let digest = message.digest();
		let signature = crypto::Signature::new(&digest, &secret_key);
		
		let vote_info: VoteMessage<AppPair> = VoteMessage{
			round_number: 1,
			block_hash: H256::from_low_u64_be(100_000),
			voter: voter_id,
			signature: signature.clone(),
		};
		assert!(signature.verify(&digest, &_public_key).is_ok());

		let hotstuff_voter: HotstuffVoter<AppPair> = HotstuffVoter{
			_phantom_data: PhantomData,
		};

		hotstuff_voter.vote_handler(&vote_info);
	}

	#[test]
	fn test_hotstuff_validate_consensus_message() {

		let key_pair = AppPair::from_seed(&[1; 32]);

		let voter_id = key_pair.public();

		// Get a keypair.
		let (_public_key, secret_key) = get_keypair();

		// Make signature.
		let message: &[u8] = b"Hello, world!";
		let digest = message.digest();
		let signature = crypto::Signature::new(&digest, &secret_key);

		let _vote_info: VoteMessage<AppPair> = VoteMessage{
			round_number: 1,
			block_hash: H256::from_low_u64_be(100_000),
			voter: voter_id,
			signature: signature.clone(),
		};

		assert!(signature.verify(&digest, &_public_key).is_ok());

		let hotstuff_voter: HotstuffVoter<AppPair> = HotstuffVoter{
			_phantom_data: PhantomData,
		};

		let consensus_msg: ConsensusMessage = ConsensusMessage{
			round_number: 1,
			parent_block_hash: H256::from_low_u64_be(100_000),
			block_hash: H256::from_low_u64_be(100_000),
		};
		let is_valid = hotstuff_voter.validate_consensus_message(&consensus_msg);
		assert_eq!(true, is_valid);
	}



	/// block produce test
    use sc_basic_authorship::ProposerFactory;
    use sp_consensus::{Environment, Proposer};
    
    use std::{sync::Arc, time::Duration};
    use substrate_test_runtime_client;
    use sc_transaction_pool::BasicPool;

    #[test]
    fn test_block_produce() {
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(
            Default::default(),
            true.into(),
            None,
            spawner.clone(),
            client.clone(),
        );
        // The first step is to create a `ProposerFactory`.
        let mut proposer_factory = ProposerFactory::new(
                spawner,
                client.clone(),
                txpool.clone(),
                None,
                None,
            );

        // From this factory, we create a `Proposer`.
        let proposer = proposer_factory.init(
            &client.header(client.chain_info().genesis_hash).unwrap().unwrap(),
        );

        // The proposer is created asynchronously.
        let proposer = futures::executor::block_on(proposer).unwrap();
    
        // This `Proposer` allows us to create a block proposition.
        // The proposer will grab transactions from the transaction pool, and put them into the block.
        let future = proposer.propose(
            Default::default(),
            Default::default(),
            Duration::from_secs(2),
            None,
        );
        
        // We wait until the proposition is performed.
        let block = futures::executor::block_on(future).unwrap();
        println!("Generated block: {:?}", block.block);

    }

}