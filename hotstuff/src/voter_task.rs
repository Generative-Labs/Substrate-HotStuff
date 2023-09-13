use futures::prelude::*;
use sc_client_api::Backend;


use crate::{
    network_bridge::{
        HotstuffNetworkBridge,
        Network as NetworkT,
        Syncing as SyncingT
    },
    LinkHalf, client::ClientForHotstuff
};
use sp_runtime::traits::Block as BlockT;


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

	let _hotstuff_network_bridge = HotstuffNetworkBridge::new(
		network.clone(),
		sync.clone(),
	);

    let future1 =  future::ready(()) ;
    let future2 =  future::ready(());

    let voter_work = future1.then(|_| future::pending::<()>());

    let telemetry_task = future2.then(|_| future::pending::<()>());


	println!("hello 🔥💃🏻 run hotstuff voting");
    Ok(future::select(voter_work, telemetry_task).map(drop))
}