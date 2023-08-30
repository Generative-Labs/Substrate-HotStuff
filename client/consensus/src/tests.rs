
use sp_consensus_hotstuff::AuthorityList;

type TestLinkHalf =
	LinkHalf<Block, PeersFullClient, LongestChain<substrate_test_runtime_client::Backend, Block>>;
type PeerData = Mutex<Option<TestLinkHalf>>;

type HotstuffPeer = Peer<PeerData, HotstuffBlockImport>;


#[derive(Default)]
struct HotstuffTestNet {
	peers: Vec<HotstuffPeer>,
	test_config: TestApi,
}

#[derive(Default, Clone)]
pub(crate) struct TestApi {
	genesis_authorities: AuthorityList,
}

impl TestApi {
	pub fn new(genesis_authorities: AuthorityList) -> Self {
		TestApi { genesis_authorities }
	}
}
