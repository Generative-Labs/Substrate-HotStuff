use std::sync::Arc;

use sc_client_api::{
    backend,
    backend::Finalizer,
    CallExecutor,
};
use sc_consensus_grandpa::{
    // LinkHalf,
    ClientForGrandpa,
};
use sp_blockchain::Error;
use sp_runtime::{
    Justification,
    traits::{Block as BlockT, NumberFor},
};

// use sc_consensus_grandpa::authorities::SharedAuthoritySet;
// use sc_service::client::Client;

pub struct FinalityBlock<B, E, Block, RA, Client> //, C, SC>
    where
        B: backend::Backend<Block>,
        E: CallExecutor<Block>,
        Block: BlockT,

{
    client: Arc<Client<B, E, Block, RA>>,
    // grandpalink: LinkHalf<Block, C, SC>,
}

impl<B, E, Block, RA, Client> FinalityBlock<B, E, Block, RA, Client>
    where
        Block: BlockT,
        B: backend::Backend<Block>,
        E: CallExecutor<Block>,
        Client: ClientForGrandpa<Block, B>,
{
    pub fn new(client: Arc<Client<B, E, Block, RA>>) -> Self {
        FinalityBlock { client }
    }

    pub fn update_state(&self, _block_hash: Block::Hash, _block_number: NumberFor<Block>) {
        // TODO: Implement your logic to update the state based on the finalized block
    }

    pub fn hotstuff_apply_finality(
        &self,
        block_hash: Block::Hash,
        persisted_justification: Option<Justification>,
        notify: bool,
    ) -> Result<(), Error> {
        // TODO: Implement the application of finality to the blockchain logic
        // self.grandpalink.client.finalize_block(block_hash, persisted_justification, notify)?;
        self.client.finalize_block(block_hash, persisted_justification, notify)?;
        Ok(())
    }
}