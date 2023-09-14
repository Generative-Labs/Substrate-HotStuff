use std::marker::PhantomData;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::sync::Arc;

use log::info;
use futures::{prelude::*, stream::StreamExt};
use sc_client_api::{Backend, BlockImportNotification};
use sc_utils::mpsc::TracingUnboundedReceiver;
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


pub fn run_hotstuff_voter<Block: BlockT,  BE: 'static, C, N, S, SC>(
    network: N,
    link: LinkHalf<Block, C, SC>,
    sync: S,
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
	);

    println!("🔥💃🏻 start run_hotstuff_voter");

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
    network: Arc<HotstuffNetworkBridge<Block,N,S>>,

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
        Self { client,network: Arc::new(network), phantom0: Default::default(), phantom1: Default::default() }
    }

    // pub fn test(&self){
    //     let topic = <<Block::Header as HeaderT>::Hashing as HashT>::hash(b"hotstuff vote");
    //     let mut gossip_msg_receiver = self.network.gossip_engine.lock().messages_for(topic);
    // }
}

impl<Block: BlockT,C, BE, N, S>  Future for SimpleVoter<Block, C, BE, N, S> 
where
    BE:Backend<Block> + 'static,
    C: ClientForHotstuff<Block, BE> + 'static,
    C::Api: sp_consensus_grandpa::GrandpaApi<Block>,
    N: NetworkT<Block> + Sync + 'static,
    S: SyncingT<Block> + Sync + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut notification: TracingUnboundedReceiver<BlockImportNotification<Block>> = self.client.import_notification_stream();
        let topic = <<Block::Header as HeaderT>::Hashing as HashT>::hash(b"hotstuff vote");
        let mut gossip_msg_receiver = self.network.gossip_engine.lock().messages_for(topic);

        loop{
            match StreamExt::poll_next_unpin(&mut notification, cx){
                Poll::Ready(None) => todo!(),
                Poll::Ready(Some(notification)) =>{
                    let header = notification.header;
                    info!("~~ voter get block from block_import, header_number {},header_hash:{}", header.number(), header.hash()); 
                    self.network.gossip_engine.lock().gossip_message(topic, header.hash().encode(), true);
                }
                Poll::Pending => {},
            }

            match StreamExt::poll_next_unpin(&mut gossip_msg_receiver, cx){
                Poll::Ready(None) => {}
                Poll::Ready(Some(notification)) => {
                    let hash: <Block::Header as HeaderT>::Hash = Decode::decode(&mut &notification.message[..]).unwrap();
                    info!("recv gossip vote message: {}", hash);
                },
                Poll::Pending => {
                    // info!("~~ gossip_msg_receiver is pending");
                },
            }
        }        
    }
}