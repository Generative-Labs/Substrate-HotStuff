use std::marker::PhantomData;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::sync::Arc;

use log::info;
use rand::Rng;

use futures::{prelude::*, stream::StreamExt};
use sc_client_api::{Backend, BlockImportNotification};
use sc_utils::mpsc::TracingUnboundedReceiver;
use sc_network::types::ProtocolName;
use sc_chain_spec::ChainSpec;

use sp_api::HeaderT;
use sp_core::{Decode ,Encode};
use sp_runtime::traits::{Block as BlockT, Hash as HashT};

use crate::{
    network_bridge::{
        HotstuffNetworkBridge,
        Network as NetworkT,
        Syncing as SyncingT
    },
    LinkHalf, client::ClientForHotstuff
};

pub(crate) const NAME: &str = "/hotstuff";

pub fn standard_name<Hash: AsRef<[u8]>>(
    genesis_hash: &Hash,
    chain_spec: &Box<dyn ChainSpec>,
) -> ProtocolName {
    let genesis_hash = genesis_hash.as_ref();
    let chain_prefix = match chain_spec.fork_id() {
        Some(fork_id) => format!("/{}/{}", array_bytes::bytes2hex("", genesis_hash), fork_id),
        None => format!("/{}", array_bytes::bytes2hex("", genesis_hash)),
    };
    format!("{}{}", chain_prefix, NAME).into()
}

pub fn hotstuff_peers_set_config(
	protocol_name: ProtocolName,
) -> sc_network::config::NonDefaultSetConfig {
	sc_network::config::NonDefaultSetConfig {
		notifications_protocol: protocol_name,
		fallback_names: Default::default(),
		// Notifications reach ~256kiB in size at the time of writing on Kusama and Polkadot.
		max_notification_size: 1024 * 1024,
		handshake: None,
		set_config: sc_network::config::SetConfig {
			in_peers: 0,
			out_peers: 0,
			reserved_nodes: Vec::new(),
			non_reserved_mode: sc_network::config::NonReservedPeerMode::Deny,
		},
	}
}

pub fn run_hotstuff_voter<Block: BlockT,  BE: 'static, C, N, S, SC>(
    network: N,
    link: LinkHalf<Block, C, SC>,
    sync: S,
    hotstuff_protocol_name: ProtocolName,
) -> sp_blockchain::Result<impl Future<Output = ()> + Send>
where
    BE: Backend<Block> + 'static,
    N: NetworkT<Block> + Sync + 'static,
    S: SyncingT<Block> + Sync + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
{
    let LinkHalf {
		client,
		select_chain,
		persistent_data,
	} = link;

	let hotstuff_network_bridge = HotstuffNetworkBridge::new(
		network.clone(),
		sync.clone(),
        hotstuff_protocol_name,  
	);

    let voter = SimpleVoter::<Block, C, BE, N, S>::new(client, hotstuff_network_bridge);

    Ok(async move{
        voter.await;
    })
}

pub struct SimpleVoter<Block: BlockT, C, BE, N, S>
where
    BE:Backend<Block> + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
    N: NetworkT<Block> + Sync + 'static,
    S: SyncingT<Block> + Sync + 'static,
{
    client: Arc<C>,
    network: HotstuffNetworkBridge<Block,N,S>,

    phantom0: PhantomData<BE>,
    phantom1: PhantomData<Block>,
}


impl<Block,C, BE, N, S> SimpleVoter<Block, C, BE, N, S> 
where
    Block: BlockT,
    BE:Backend<Block> + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
    N: NetworkT<Block> + Sync + 'static,
    S: SyncingT<Block> + Sync + 'static,
{
    pub fn new(
        client: Arc<C>,
        network: HotstuffNetworkBridge<Block, N, S>,
    )->Self{
        Self { client, network, phantom0: Default::default(), phantom1: Default::default() }
    }

    pub fn do_finalize_block(
        &self,
        hash: Block::Hash,
    ){
        match self.client.finalize_block(hash, None, false){
            Ok(_) => info!("~~ Simple voter finalize block success {}", hash),
            Err(e) => info!("~~ Simple voter finalize block success {}, error{}", hash, e),
        }
    }
}

// TestMessage will be replace hotstuff protocol message
#[derive(Encode, Decode, Debug)]
pub struct TestMessage<Block: BlockT>{
    pub topic: Block::Hash,
    pub hash: Vec<u8>,
    pub id: u64,
}

impl<Block: BlockT,C, BE, N, S> Future for SimpleVoter<Block, C, BE, N, S> 
where
    BE:Backend<Block> + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
    N: NetworkT<Block> + Sync,
    S: SyncingT<Block> + Sync + 'static,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut notification: TracingUnboundedReceiver<BlockImportNotification<Block>> = self.client.import_notification_stream();
        let topic = <<Block::Header as HeaderT>::Hashing as HashT>::hash(b"hotstuff/vote");

        info!("&&&!!~~ hotstuff vote topic {}", topic);

        let mut gossip_msg_receiver = self.network.gossip_engine.lock().messages_for(topic);
        let mut rng = rand::thread_rng();

        loop{
            match StreamExt::poll_next_unpin(&mut notification, cx){
                Poll::Ready(None) => {},
                Poll::Ready(Some(notification)) =>{
                    let header = notification.header;
                    info!("~~ voter get block from block_import, header_number {},header_hash:{}", header.number(), header.hash()); 
                    
                    // If the gossip engine detects that a message received from the network has already been registered 
                    // or is pending broadcast, it will not be reported to the upper-level receivers.
                    // The use of random numbers here is only for testing purposes.
                    let id: u64 = rng.gen_range(1..=100000); 
                    let message = TestMessage::<Block>{
                        topic,
                        hash: header.hash().encode().to_vec(),
                        id,
                    };

                    info!("&&&!!~~ hotstuff vote topic {}, message {:#?}", topic, message.encode());
                    self.network.gossip_engine.lock().register_gossip_message(topic, message.encode());
                }
                Poll::Pending => {},
            }
            
            match StreamExt::poll_next_unpin(&mut gossip_msg_receiver, cx){
                Poll::Ready(None) => {}
                Poll::Ready(Some(notification)) => {
                    let message: TestMessage::<Block> = Decode::decode(&mut &notification.message[..]).unwrap();
                    info!("~~ recv gossip vote message: {:#?}", message);
                    match <<Block as BlockT>::Header as HeaderT>::Hash::decode(&mut &message.hash[..]){
                        Ok(hash) => self.do_finalize_block(hash),
                        Err(e) => {
                            info!(" decode TestMessage hash failed: {:#?}", e);                            
                        },
                    };
                },
                Poll::Pending => {},
            };

            let _ = Future::poll(Pin::new(&mut self.network), cx);
        }   
    }
}

impl<Block: BlockT,C, BE, N, S> Unpin for SimpleVoter<Block, C, BE, N, S> 
where
    BE:Backend<Block> + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
    N: NetworkT<Block> + Sync,
    S: SyncingT<Block> + Sync + 'static,
{}
