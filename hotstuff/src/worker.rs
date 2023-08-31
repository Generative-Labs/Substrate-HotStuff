use futures::prelude::*;

use sp_api::ProvideRuntimeApi;
use sc_client_api::{backend::AuxStore, BlockOf};

use sp_runtime::traits::{Block as BlockT, Member, NumberFor};
use codec::Codec;
use sp_blockchain::HeaderBackend;
pub use sp_consensus::SyncOracle;

use sp_consensus::{ Environment, Error as ConsensusError, Proposer, SelectChain};
use sc_consensus::BlockImport;


use tokio::time::{sleep, Duration};
use sp_core::crypto::Pair;
use sp_application_crypto::AppPublic;

use sp_consensus_hotstuff;

use crate::params::StartHotstuffParams;

use sp_consensus_hotstuff::HotstuffApi;

type AuthorityId<P> = <P as Pair>::Public;

struct HotstuffWork {
}

impl HotstuffWork {
    async fn do_work(&self) {
        // Implement your actual work logic here
        println!("Performing HotstuffWork...");
    }
}


pub fn start_hotstuff<P, B, C, SC, I, PF, SO, L, Error>(
	StartHotstuffParams {
		client: _,
		select_chain: _,
		block_import: _,
		proposer_factory: _,
		sync_oracle: _,
		justification_sync_link: _,
		force_authoring: _,
		keystore: _,
		telemetry: _,
		compatibility_mode: _,
	}: StartHotstuffParams<C, SC, I, PF, SO, L, NumberFor<B>>,
) -> Result<impl Future<Output = ()>, ConsensusError> 
where
	P: Pair,
	P::Public: AppPublic + Member,
	P::Signature: TryFrom<Vec<u8>> + Member + Codec,
	B: BlockT,
	C: ProvideRuntimeApi<B> + BlockOf + AuxStore + HeaderBackend<B> + Send + Sync,
	C::Api: HotstuffApi<B, AuthorityId<P>>,
	SC: SelectChain<B>,
	I: BlockImport<B, Transaction = sp_api::TransactionFor<C, B>> + Send + Sync + 'static,
	PF: Environment<B, Error = Error> + Send + Sync + 'static,
	PF::Proposer: Proposer<B, Error = Error, Transaction = sp_api::TransactionFor<C, B>>,
	SO: SyncOracle + Send + Sync + Clone,
	L: sc_consensus::JustificationSyncLink<B>,
	Error: std::error::Error + Send + From<ConsensusError> + 'static,
{
    println!("Hello hotstuff");

    let work = HotstuffWork {};

    let future_work = async move {
        loop {
            work.do_work().await;
            sleep(Duration::from_millis(3000)).await;
            println!("3000 ms have elapsed");
        }
    };

    Ok(future_work)
}


#[cfg(test)]
mod tests {
	use crypto::Digest;
	use ed25519_dalek::Digest as _;
	use ed25519_dalek::Sha512;

	use sp_core::H256;

	use std::marker::PhantomData;

	use sp_application_crypto::Pair;
	use sp_application_crypto::ed25519::AppPair;

	use crate::voter::{
		HotstuffVoter,
		leader::HotstuffLeader,validator::HotstuffValidator,
		message::VoteMessage,  message::ConsensusMessage
	};

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

}