
// pub mod block_producer;
// pub mod finalize_block;
pub mod leader;
pub mod message;
pub mod params;

pub mod qc;
pub mod types;
pub mod validator;
pub mod voter;
pub mod aux_schema;
// pub mod voting_handler;
pub mod worker;
pub mod import;
pub use import::HotstuffBlockImport;
pub mod network_bridge;
pub mod client;
pub mod voter_task;
pub mod authorities;
pub use client::{ block_import, LinkHalf };

/// The log target to be used by client code.
pub const CLIENT_LOG_TARGET: &str = "hotstuff";

pub use voter_task::run_hotstuff_voter;
pub use authorities::SharedAuthoritySet;

pub enum HotstuffError {
	Other(String)
}

#[cfg(test)]
mod tests {
	use crypto::Digest;
	use ed25519_dalek::Digest as _;
	use ed25519_dalek::Sha512;

// 	use futures::channel::mpsc;
// use libp2p_identity::Keypair;
// use libp2p_identity::PeerId;
// use sc_network_gossip::GossipEngine;
// use sp_api::ApiRef;
// use sp_api::ProvideRuntimeApi;
use sp_core::H256;

	use std::marker::PhantomData;

	use sp_application_crypto::Pair;
	use sp_application_crypto::ed25519::AppPair;

// 	use crate::import::GossipValidator;
// use crate::import::KnownPeers;
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

	// gossip test
	use sc_network::ProtocolName;
	use array_bytes::bytes2hex;

	// const GENESIS_HASH: H256 = H256::zero();
	const GOSSIP_NAME: &str = "/hotstuff/2";

	// use sc_network_test::{Block, Peer, PeersClient, BlockImportAdapter};

	#[allow(unused)]
	pub fn gossip_protocol_name<Hash: AsRef<[u8]>>(
		genesis_hash: Hash,
		fork_id: Option<&str>,
	) -> ProtocolName {
		let genesis_hash = genesis_hash.as_ref();
		if let Some(fork_id) = fork_id {
			format!("/{}/{}{}", bytes2hex("", genesis_hash), fork_id, GOSSIP_NAME).into()
		} else {
			format!("/{}{}", bytes2hex("", genesis_hash), GOSSIP_NAME).into()
		}
	}


	#[derive(Clone)]
	pub(crate) struct TestApi {
		pub _hotstuff_genesis: u64,
	}

	impl TestApi {

		#[allow(unused)]
		pub fn new(
			_hotstuff_genesis: u64,
		) -> Self {
			TestApi {
				_hotstuff_genesis,
			}
		}
	}

	// fn hotstuff_gossip_proto_name() -> ProtocolName {
	// 	gossip_protocol_name(GENESIS_HASH, None)
	// }
	// pub(crate) type HotstuffPeer = Peer<PeerData, HotstuffBlockImport>;



	// pub struct TestNetwork {
	// 	peer_id: PeerId,
	// 	identity: Keypair,
	// }
	// impl Default for TestNetwork {
	// 	fn default() -> Self {
	// 		let (tx, rx) = mpsc::unbounded();
	// 		let identity = Keypair::generate_ed25519();
	// 		TestNetwork {
	// 			peer_id: identity.public().to_peer_id(),
	// 			identity,
	// 		}
	// 	}
	// }
	

	#[derive(Default)]
	pub(crate) struct PeerData {
	}
	// use parking_lot::{Mutex};
	// #[test]
    // fn test_gossip_engine() {
	// 	let network = Arc::new(TestNetwork::default());

    //     // let client = Arc::new(substrate_test_runtime_client::new());

	// 	let known_peers = Arc::new(Mutex::new(KnownPeers::new()));

	// 	let (gossip_validator, _) = GossipValidator::new(known_peers);
	// 	let gossip_validator = Arc::new(gossip_validator);

	// 	let mut gossip_engine = GossipEngine::new(
	// 		network.clone(),
	// 		sync.clone(),
	// 		"/hotstuff/whatever",
	// 		gossip_validator.clone(),
	// 		None,
	// 	);

	// 	gossip_engine.gossip_message("gossiptest", vec![1, 2], false);
    // }

}