use std::collections::HashSet;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::sync::Arc;

use log::{warn, info};
use rand::Rng;

use futures::{prelude::*, stream::StreamExt};
use sc_client_api::{Backend, BlockImportNotification};
use sc_utils::mpsc::TracingUnboundedReceiver;
use sc_network::types::ProtocolName;
use sc_chain_spec::ChainSpec;
use sc_client_api::CallExecutor;

use sp_consensus_hotstuff::{sr25519::AuthorityId as HotstuffId, HOTSTUFF_KEY_TYPE, HOTSTUFF_ENGINE_ID, Slot};
use sp_api::HeaderT;
use sp_core::{Decode ,Encode, traits::CallContext, ByteArray};
use sp_runtime::traits::{Block as BlockT, Hash as HashT};
use sp_keystore::KeystorePtr;

use crate::{
    network_bridge::{
        HotstuffNetworkBridge,
        Network as NetworkT,
        Syncing as SyncingT
    },
    LinkHalf, client::ClientForHotstuff,
    HotstuffError,
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
    key_store: KeystorePtr,
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

    let voter = SimpleVoter::<Block, C, BE, N, S>::new(client, hotstuff_network_bridge, key_store);

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
    voted_block_set: HashSet<Block::Hash>,
    key_store: KeystorePtr,

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
        key_store: KeystorePtr
    )->Self{
        Self { client, network, voted_block_set: HashSet::default() , key_store, phantom0: Default::default(), phantom1: Default::default() }
    }

    pub fn do_finalize_block(
        &self,
        hash: Block::Hash,
    ){
        match self.client.finalize_block(hash, None, false){
            Ok(_) => info!("~~ Simple voter finalize block success {}", hash),
            Err(e) => warn!("~~ Simple voter finalize block success {}, error{}", hash, e),
        }
    }

    pub fn get_block_author(&self, block_header: &Block::Header){
        let authorities = self
            .client.executor()
            .call(block_header.hash(), "HotstuffApi_authorities", &[], CallContext::Offchain)
            .ok();

        if let Some(sets) = authorities{
            let ids: Vec::<HotstuffId> = Decode::decode(&mut &sets[..]).unwrap();

            let digest = block_header.digest();
            for iter in digest.logs(){
                if let Some(mut cons) = iter.as_consensus(){
                    log::info!("~~~ consensus {:#?}", cons);
                    if cons.0 == HOTSTUFF_ENGINE_ID {
                        let slot = Slot::decode(&mut cons.1).unwrap();
                        let author_index = *slot % ids.len() as u64;
                        info!("get block author index: {}", author_index);
                    }
                }

                if let Some(mut cons) = iter.as_pre_runtime(){
                    log::info!("~~~ consensus {:#?}", cons);
                    if cons.0 == HOTSTUFF_ENGINE_ID {
                        let slot = Slot::decode(&mut cons.1).unwrap();
                        let author_index = *slot % ids.len() as u64;
                        info!("get block author index: {}", author_index);
                        let res = self.key_store.has_keys(&[(ids[author_index as usize].to_raw_vec(), HOTSTUFF_KEY_TYPE)]);
                        if res{
                            info!("~~~ im author");
                        }
                    }
                }
            }    
        }  
    }

    fn is_block_author(&self, block_header: &Block::Header)->Result<bool, HotstuffError>{
        let authorities_data = self
            .client.executor()
            .call(
                block_header.hash(),
                "HotstuffApi_authorities",
                &[], 
                CallContext::Offchain)
            .map_err(|e| HotstuffError::Other(e.to_string()))?;

        let authorities: Vec::<HotstuffId> = Decode::decode(&mut &authorities_data[..])
            .map_err(|e| HotstuffError::Other(e.to_string()))?;

        for item in block_header.digest().logs(){
            if let Some(mut cons) = item.as_pre_runtime(){
                // just process block generate by hotstuff(aura)
                if cons.0 != HOTSTUFF_ENGINE_ID {
                    continue;
                }

                let slot = Slot::decode(&mut cons.1)
                        .map_err(|e|HotstuffError::Other(e.to_string()))?;

                let author_index = *slot % authorities.len() as u64;
                
                if self.key_store.has_keys(&[(authorities[author_index as usize].to_raw_vec(), HOTSTUFF_KEY_TYPE)]){
                    info!("i'm author for block {}", block_header.number());
                    return Ok(true);
                }
            }   
        }

        Err(HotstuffError::Other("unknown block author".to_string()))
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

        let mut gossip_msg_receiver = self.network.gossip_engine.lock().messages_for(topic);
        let mut rng = rand::thread_rng();

        loop{
            match StreamExt::poll_next_unpin(&mut notification, cx){
                Poll::Ready(None) => {
                    break;
                },
                Poll::Ready(Some(notification)) =>{
                    let header = notification.header;
                    info!("~~ Simple voter get block from block_import, header_number {},header_hash:{}", header.number(), header.hash()); 
                    
                    // If the gossip engine detects that a message received from the network has already been registered 
                    // or is pending broadcast, it will not be reported to the upper-level receivers.
                    // The use of random numbers here is only for testing purposes.
                    let id: u64 = rng.gen_range(1..=100000); 
                    let message = TestMessage::<Block>{
                        topic,
                        hash: header.hash().encode().to_vec(),
                        id,
                    };

                    self.is_block_author(&header);

                    self.network.gossip_engine.lock().register_gossip_message(topic, message.encode());
                }
                Poll::Pending => {},
            }
            
            match StreamExt::poll_next_unpin(&mut gossip_msg_receiver, cx){
                Poll::Ready(None) => {
                    break;
                }
                Poll::Ready(Some(notification)) => {
                    let message: TestMessage::<Block> = Decode::decode(&mut &notification.message[..]).unwrap();
                    match <<Block as BlockT>::Header as HeaderT>::Hash::decode(&mut &message.hash[..]){
                        Ok(hash) => {
                            if !self.voted_block_set.contains(&hash){
                                self.voted_block_set.insert(hash);
                                self.do_finalize_block(hash)
                            }
                        },
                        Err(e) => {
                            warn!(" decode `TestMessage` hash failed: {:#?}", e);                            
                        },
                    };
                },
                Poll::Pending => {},
            };

            match Future::poll(Pin::new(&mut self.network), cx){
                Poll::Ready(_) => {
                    break;
                },
                Poll::Pending => {},
            }
        } 
        Poll::Ready(())  
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
