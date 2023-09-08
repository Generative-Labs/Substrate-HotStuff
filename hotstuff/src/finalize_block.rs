use std::sync::Arc;

use sc_client_api::{
    backend,
    backend::Finalizer,
    CallExecutor,
};
use sc_consensus_grandpa::{
    ClientForGrandpa,
};
use sp_blockchain::Error;
use sp_runtime::{
    Justification,
    traits::{Block as BlockT, NumberFor},
};

pub struct FinalityBlock<B, E, Block, RA, Client>
    where
        B: backend::Backend<Block>,
        E: CallExecutor<Block>,
        Block: BlockT,
{
    client: Arc<Client>, // Use the generic Client type here
    _unused_b: std::marker::PhantomData<B>,
    _unused_e: std::marker::PhantomData<E>,
    _unused_block: std::marker::PhantomData<Block>,
    _unused_ra: std::marker::PhantomData<RA>,
}

impl<B, E, Block, RA, Client> FinalityBlock<B, E, Block, RA, Client>
    where
        Block: BlockT,
        B: backend::Backend<Block>,
        E: CallExecutor<Block>,
        Client: ClientForGrandpa<Block, B>,
{
    pub fn new(client: Arc<Client>) -> Self {
        FinalityBlock {
            client,
            _unused_b: Default::default(),
            _unused_e: Default::default(),
            _unused_block: Default::default(),
            _unused_ra: Default::default(),
        }
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
        self.client.finalize_block(block_hash, persisted_justification, notify)?;
        Ok(())
    }
}