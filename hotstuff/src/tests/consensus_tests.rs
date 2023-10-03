// hotstuff worker tests

use super::*;

use std::sync::Mutex;

use sc_consensus::{BoxJustificationImport, LongestChain};
use sc_network_test::{
	Block, BlockImportAdapter, FullPeerConfig, Hash, PassThroughVerifier, Peer, PeersClient,
	PeersFullClient, TestClient, TestNetFactory,
};
use sp_api::{ApiRef, ProvideRuntimeApi};
use sp_consensus_hotstuff::HotstuffApi;

use crate::client::GenesisAuthoritySetProvider;

type TestLinkHalf =
	LinkHalf<Block, PeersFullClient, LongestChain<substrate_test_runtime_client::Backend, Block>>;
type PeerData = Mutex<Option<TestLinkHalf>>;
type HotstuffBlockImport = crate::import::HotstuffBlockImport<
	substrate_test_runtime_client::Backend,
	Block,
	PeersFullClient,
>;
type GrandpaPeer = Peer<PeerData, HotstuffBlockImport>;

#[derive(Default, Clone)]
pub(crate) struct TestApi {
	genesis_authorities: AuthorityList,
}

impl TestApi {
	pub fn new(genesis_authorities: AuthorityList) -> Self {
		TestApi { genesis_authorities }
	}
}

pub(crate) struct RuntimeApi {
	inner: TestApi,
}

impl ProvideRuntimeApi<Block> for TestApi {
	type Api = RuntimeApi;

	fn runtime_api(&self) -> ApiRef<'_, Self::Api> {
		RuntimeApi { inner: self.clone() }.into()
	}
}

impl GenesisAuthoritySetProvider<Block> for TestApi {
	fn get(&self) -> sp_blockchain::Result<AuthorityList> {
		let x = self
			.genesis_authorities
			.iter()
			.map(|(id, _)| id.clone())
			.collect::<Vec<AuthorityId>>();

		Ok(self.genesis_authorities.clone())
	}
}

sp_api::mock_impl_runtime_apis! {
	impl HotstuffApi<Block, AuthorityId> for RuntimeApi {
		fn slot_duration() -> sp_consensus_hotstuff::SlotDuration {
			sp_consensus_hotstuff::SlotDuration::from_millis(1000)
		}

		fn authorities() -> Vec<AuthorityId> {
			self.inner
				.genesis_authorities
				.iter()
				.map(|(id, _)| id.clone())
				.collect::<Vec<AuthorityId>>()
		}
	}
}

#[derive(Default)]
struct GrandpaTestNet {
	peers: Vec<GrandpaPeer>,
	test_config: TestApi,
}

impl GrandpaTestNet {
	fn new(test_config: TestApi, n_authority: usize, n_full: usize) -> Self {
		let mut net =
			GrandpaTestNet { peers: Vec::with_capacity(n_authority + n_full), test_config };

		for _ in 0..n_authority {
			net.add_authority_peer();
		}

		for _ in 0..n_full {
			net.add_full_peer();
		}

		net
	}
}

impl GrandpaTestNet {
	fn add_authority_peer(&mut self) {
		self.add_full_peer_with_config(FullPeerConfig {
			notifications_protocols: vec![crate::config::HOTSTUFF_PROTOCOL_NAME.into()],
			is_authority: true,
			..Default::default()
		})
	}
}

impl TestNetFactory for GrandpaTestNet {
	type Verifier = PassThroughVerifier;
	type PeerData = PeerData;
	type BlockImport = HotstuffBlockImport;

	fn add_full_peer(&mut self) {
		self.add_full_peer_with_config(FullPeerConfig {
			notifications_protocols: vec![crate::config::HOTSTUFF_PROTOCOL_NAME.into()],
			is_authority: false,
			..Default::default()
		})
	}

	fn make_verifier(&self, _client: PeersClient, _: &PeerData) -> Self::Verifier {
		PassThroughVerifier::new(false) // use non-instant finality.
	}

	fn make_block_import(
		&self,
		client: PeersClient,
	) -> (BlockImportAdapter<Self::BlockImport>, Option<BoxJustificationImport<Block>>, PeerData) {
		let (client, backend) = (client.as_client(), client.as_backend());
		let (import, link) = crate::client::block_import(client.clone(), &self.test_config)
			.expect("Could not create block import for fresh peer.");
		let justification_import = Box::new(import.clone());
		(BlockImportAdapter::new(import), Some(justification_import), Mutex::new(Some(link)))
	}

	fn peer(&mut self, i: usize) -> &mut GrandpaPeer {
		&mut self.peers[i]
	}

	fn peers(&self) -> &Vec<GrandpaPeer> {
		&self.peers
	}

	fn peers_mut(&mut self) -> &mut Vec<GrandpaPeer> {
		&mut self.peers
	}

	fn mut_peers<F: FnOnce(&mut Vec<GrandpaPeer>)>(&mut self, closure: F) {
		closure(&mut self.peers);
	}
}

// #[tokio::test]
