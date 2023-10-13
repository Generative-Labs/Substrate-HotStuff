// hotstuff worker tests
use super::*;

use futures::{future, stream, FutureExt};
use parking_lot::{Mutex, RwLock};
use tokio::runtime::Handle;

use sc_client_api::{Finalizer, HeaderBackend};
use sc_consensus::{BoxJustificationImport, LongestChain};
use sc_network_test::{
	Block, BlockImportAdapter, FullPeerConfig, PassThroughVerifier, Peer, PeersClient,
	PeersFullClient, TestNetFactory,
};
use sp_api::{ApiRef, ProvideRuntimeApi};
use sp_keyring::Sr25519Keyring;
use sp_keystore::{testing::MemoryKeystore, Keystore, KeystorePtr};
use sp_runtime::traits::Header as HeaderT;
use substrate_test_runtime::Hashing;

use crate::client::GenesisAuthoritySetProvider;
use sp_consensus_hotstuff::HotstuffApi;

type TestLinkHalf =
	LinkHalf<Block, PeersFullClient, LongestChain<substrate_test_runtime_client::Backend, Block>>;
type PeerData = Mutex<Option<TestLinkHalf>>;
type HotstuffBlockImport = crate::import::HotstuffBlockImport<
	substrate_test_runtime_client::Backend,
	Block,
	PeersFullClient,
>;
type HotstuffPeer = Peer<PeerData, HotstuffBlockImport>;

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
	_inner: TestApi,
}

impl ProvideRuntimeApi<Block> for TestApi {
	type Api = RuntimeApi;

	fn runtime_api(&self) -> ApiRef<'_, Self::Api> {
		RuntimeApi { _inner: self.clone() }.into()
	}
}

impl GenesisAuthoritySetProvider<Block> for TestApi {
	fn get(&self) -> sp_blockchain::Result<Vec<AuthorityId>> {
		Ok(self.genesis_authorities.iter().map(|a| a.0.clone()).collect())
	}
}

#[derive(Default)]
struct TestNet {
	peers: Vec<HotstuffPeer>,
	test_config: TestApi,
}

sp_api::mock_impl_runtime_apis! {
	impl HotstuffApi<Block, AuthorityId> for RuntimeApi {
		fn slot_duration() -> sp_consensus_hotstuff::SlotDuration {
			// sp_consensus_hotstuff::SlotDuration::from_millis(Hotstuff::slot_duration())
			unimplemented!()
		}

		fn authorities() -> Vec<AuthorityId> {
			unimplemented!()
		}
	}
}

impl TestNet {
	fn new(test_config: TestApi, n_authority: usize, n_full: usize) -> Self {
		let mut net = TestNet { peers: Vec::with_capacity(n_authority + n_full), test_config };

		for _ in 0..n_authority {
			net.add_authority_peer();
		}

		for _ in 0..n_full {
			net.add_full_peer();
		}

		net
	}
}

impl TestNet {
	fn add_authority_peer(&mut self) {
		self.add_full_peer_with_config(FullPeerConfig {
			notifications_protocols: vec![crate::config::HOTSTUFF_PROTOCOL_NAME.into()],
			is_authority: true,
			..Default::default()
		})
	}
}

impl TestNetFactory for TestNet {
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
		let (client, _backend) = (client.as_client(), client.as_backend());
		let (import, link) = crate::client::block_import(client.clone(), &self.test_config)
			.expect("Could not create block import for fresh peer.");
		let justification_import = Box::new(import.clone());
		(BlockImportAdapter::new(import), Some(justification_import), Mutex::new(Some(link)))
	}

	fn peer(&mut self, i: usize) -> &mut HotstuffPeer {
		&mut self.peers[i]
	}

	fn peers(&self) -> &Vec<HotstuffPeer> {
		&self.peers
	}

	fn peers_mut(&mut self) -> &mut Vec<HotstuffPeer> {
		&mut self.peers
	}

	fn mut_peers<F: FnOnce(&mut Vec<HotstuffPeer>)>(&mut self, closure: F) {
		closure(&mut self.peers);
	}
}

fn make_ids(keys: &[Sr25519Keyring]) -> AuthorityList {
	keys.iter().map(|&key| key.public().into()).map(|id| (id, 1)).collect()
}

fn create_keystore(authority: Sr25519Keyring) -> KeystorePtr {
	let keystore = MemoryKeystore::new();
	keystore
		.sr25519_generate_new(AuthorityId::ID, Some(&authority.to_seed()))
		.expect("Creates authority key");
	keystore.into()
}

fn start_hotstuff<B, BE, C, N, S, SC>(
	network: N,
	link: LinkHalf<B, C, SC>,
	sync: S,
	hotstuff_protocol_name: ProtocolName,
	keystore: KeystorePtr,
	authorities: AuthorityList,
) -> sp_blockchain::Result<(impl Future<Output = ()> + Send, impl Future<Output = ()> + Send)>
where
	B: BlockT,
	BE: Backend<B> + 'static,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
	C: ClientForHotstuff<B, BE> + 'static,
{
	let (worker, net) = build_hotstuff_components(
		network,
		link,
		sync,
		hotstuff_protocol_name,
		keystore,
		authorities,
	)?;
	Ok((async { worker.run().await }, net))
}

fn build_hotstuff_components<B, BE, C, N, S, SC>(
	network: N,
	link: LinkHalf<B, C, SC>,
	sync: S,
	hotstuff_protocol_name: ProtocolName,
	keystore: KeystorePtr,
	authorities: AuthorityList,
) -> sp_blockchain::Result<(ConsensusWorker<B, BE, C, N, S>, ConsensusNetwork<B, N, S>)>
where
	B: BlockT,
	BE: Backend<B> + 'static,
	N: NetworkT<B> + Sync + 'static,
	S: SyncingT<B> + Sync + 'static,
	C: ClientForHotstuff<B, BE> + 'static,
{
	let LinkHalf { client, .. } = link;

	let network = HotstuffNetworkBridge::new(network.clone(), sync.clone(), hotstuff_protocol_name);
	let synchronizer = Synchronizer::<B, BE, C>::new(client.clone());
	let consensus_state = ConsensusState::<B>::new(keystore, authorities);

	let (consensus_msg_tx, consensus_msg_rx) = channel::<ConsensusMessage<B>>(1000);

	let queue = PendingFinalizeBlockQueue::<B>::new(client.clone()).expect("error");

	let consensus_worker = ConsensusWorker::<B, BE, C, N, S>::new(
		consensus_state,
		client,
		network.clone(),
		synchronizer,
		2000,
		consensus_msg_tx.clone(),
		consensus_msg_rx,
		queue.queue(),
	);

	let consensus_network = ConsensusNetwork::<B, N, S>::new(network, consensus_msg_tx, queue);

	Ok((consensus_worker, consensus_network))
}

fn instantiate_hotstuff(net: &mut TestNet, peers: &[Sr25519Keyring]) -> impl Future<Output = ()> {
	let voters = stream::FuturesUnordered::new();
	let authority_list = make_ids(peers);
	for (peer_id, key) in peers.iter().enumerate() {
		let keystore = create_keystore(*key);

		let (net_service, link) = {
			// temporary needed for some reason
			let link =
				net.peers[peer_id].data.lock().take().expect("link initialized at startup; qed");
			(net.peers[peer_id].network_service().clone(), link)
		};
		let sync = net.peers[peer_id].sync_service().clone();

		let (v0, v1) = start_hotstuff(
			net_service,
			link,
			sync,
			crate::config::HOTSTUFF_PROTOCOL_NAME.into(),
			keystore,
			authority_list.clone(),
		)
		.expect("");

		fn assert_send<T: Send>(_: &T) {}
		assert_send(&v0);
		assert_send(&v1);

		let v = futures::future::join(Box::pin(v0), v1);
		voters.push(v);
	}

	voters.for_each(|_| async move {})
}

async fn run_until_complete(future: impl Future + Unpin, net: &Arc<Mutex<TestNet>>) {
	let drive_to_completion = futures::future::poll_fn(|cx| {
		net.lock().poll(cx);
		Poll::<()>::Pending
	});
	future::select(future, drive_to_completion).await;
}

// run the voters to completion. provide a closure to be invoked after
// the voters are spawned but before blocking on them.
async fn run_to_completion_with<F>(
	blocks: u64,
	net: Arc<Mutex<TestNet>>,
	peers: &[Sr25519Keyring],
	with: F,
) -> u64
where
	F: FnOnce(Handle) -> Option<Pin<Box<dyn Future<Output = ()>>>>,
{
	let mut wait_for = Vec::new();

	let highest_finalized = Arc::new(RwLock::new(0));

	if let Some(f) = (with)(Handle::current()) {
		wait_for.push(f);
	};

	for (peer_id, _) in peers.iter().enumerate() {
		let highest_finalized = highest_finalized.clone();
		let client = net.lock().peers[peer_id].client().clone();

		wait_for.push(Box::pin(
			client
				.finality_notification_stream()
				.take_while(move |n| {
					let mut highest_finalized = highest_finalized.write();
					if *n.header.number() > *highest_finalized {
						*highest_finalized = *n.header.number();
					}
					future::ready(n.header.number() < &blocks)
				})
				.collect::<Vec<_>>()
				.map(|_| ()),
		));
	}

	// wait for all finalized on each.
	let wait_for = ::futures::future::join_all(wait_for);

	run_until_complete(wait_for, &net).await;
	let highest_finalized = *highest_finalized.read();
	highest_finalized
}

async fn run_to_completion(blocks: u64, net: Arc<Mutex<TestNet>>, peers: &[Sr25519Keyring]) -> u64 {
	run_to_completion_with(blocks, net, peers, |_| None).await
}

// Test when there are three voter, they can finalize block.
#[tokio::test]
async fn finalize_three_voters() {
	sp_tracing::try_init_simple();

	let peers = &[Sr25519Keyring::Alice, Sr25519Keyring::Bob, Sr25519Keyring::Charlie];
	let voters = make_ids(peers);

	let mut net = TestNet::new(TestApi::new(voters), 3, 0);
	tokio::spawn(instantiate_hotstuff(&mut net, peers));

	net.peer(0).push_blocks(10, false);
	net.run_until_sync().await;

	let net = Arc::new(Mutex::new(net));
	run_to_completion(10, net.clone(), peers).await;

	for i in 0..3 {
		assert_eq!(net.lock().peer(i).client().info().finalized_number as u64, 10);
	}
}

// Test when there are three voter and a full node, they can finalize block.
#[tokio::test]
async fn finalize_3_voters_with_1_full() {
	sp_tracing::try_init_simple();

	let peers = &[
		Sr25519Keyring::Alice,
		Sr25519Keyring::Bob,
		Sr25519Keyring::Charlie,
		Sr25519Keyring::Dave,
	];
	let voters = make_ids(peers);

	let mut net = TestNet::new(TestApi::new(voters), 3, 1);
	tokio::spawn(instantiate_hotstuff(&mut net, peers));

	net.peer(0).push_blocks(10, false);
	net.run_until_sync().await;

	let net = Arc::new(Mutex::new(net));
	run_to_completion(10, net.clone(), peers).await;

	for i in 0..4 {
		assert_eq!(net.lock().peer(i).client().info().finalized_number as u64, 10);
	}
}

#[tokio::test]
async fn single_voter_get_proposal_block() {
	sp_tracing::try_init_simple();

	let peers = &[Sr25519Keyring::Alice];
	let peer_id = 0;
	let voters = make_ids(peers);
	let mut net = TestNet::new(TestApi::new(voters.clone()), 3, 1);
	let keystore = create_keystore(peers[peer_id]);
	let (net_service, link) = {
		let link = net.peers[peer_id].data.lock().take().expect("link initialized at startup; qed");
		(net.peers[peer_id].network_service().clone(), link)
	};
	let client = link.client.clone();
	let sync = net.peers[peer_id].sync_service().clone();

	let (mut hotstuff_worker, hotstuff_network) = build_hotstuff_components(
		net_service,
		link,
		sync,
		crate::config::HOTSTUFF_PROTOCOL_NAME.into(),
		keystore,
		voters,
	)
	.unwrap();

	tokio::spawn(hotstuff_network);

	// scenario 1: no import block, if get a empty payload,
	let empty_payload_hash = Hashing::hash(EMPTY_PAYLOAD);
	assert_eq!(hotstuff_worker.get_proposal_block(), Some(empty_payload_hash));

	// scenario 2: import 2 block, then call get_proposal_block twice.
	// first, it return the imported block
	// second, it return a empty payload
	net.peer(0).push_blocks(2, false);
	net.run_until_sync().await;

	let block_hahs1 = client.hash(1).unwrap().unwrap();
	assert_eq!(hotstuff_worker.get_proposal_block(), Some(block_hahs1));
	assert_eq!(hotstuff_worker.get_proposal_block(), Some(empty_payload_hash));

	// after client finalize block 1. the get_proposal_block will return block2
	client.finalize_block(block_hahs1, None, true).expect("finalize block1");
	assert_eq!(hotstuff_worker.get_proposal_block(), Some(client.hash(2).unwrap().unwrap()));

	// after client finalize block 2. the get_proposal_block will return empty payload
	client.finalize_block(block_hahs1, None, true).expect("finalize block2");
	assert_eq!(hotstuff_worker.get_proposal_block(), Some(empty_payload_hash));
}
