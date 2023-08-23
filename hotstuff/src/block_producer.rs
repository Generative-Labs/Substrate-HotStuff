use futures::channel::oneshot;
use log::{debug, error, info, trace, warn};
use sc_block_builder::{BlockBuilderApi, BlockBuilderProvider};
use sc_client_api::backend;
use sc_transaction_pool_api::{InPoolTransaction, TransactionPool};
use sp_blockchain::{
    ApplyExtrinsicFailed::Validity, Error::ApplyExtrinsicFailed, HeaderBackend,
};
use sp_consensus::Proposal;
use sp_core::traits::SpawnNamed;
use sp_inherents::InherentData;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use std::{marker::PhantomData, pin::Pin, sync::Arc, time};

/// The BlockProducer struct responsible for generating blocks.
pub struct BlockProducer<A, B, C, PR> {
    proposer_factory: ProposerFactory<A, B, C, PR>,
    // todo : Other fields
}

impl<A, B, C, PR> BlockProducer<A, B, C, PR> {
    /// Create a new BlockProducer instance.
    pub fn new(
        spawn_handle: impl SpawnNamed + 'static,
        client: Arc<C>,
        transaction_pool: Arc<A>,
        proposer_factory: ProposerFactory<A, B, C, PR>,
    ) -> Self {
        BlockProducer {
            proposer_factory,
        }
    }

    /// Generate a block using the proposer factory.
    pub fn generate_block(
        &self,
        inherent_data: InherentData,
        inherent_digests: Digest,
        max_duration: time::Duration,
        block_size_limit: Option<usize>,
    ) -> Result<
        Proposal<B, backend::TransactionFor<B, Block>, PR::Proof>,
        BlockchainError,
    > {
        let proposer = self
            .proposer_factory
            .init_with_now(parent_header, Box::new(time::Instant::now));
        let proposal = proposer.propose(
            inherent_data,
            inherent_digests,
            max_duration,
            block_size_limit,
        );
        proposal
    }

    // todo: Other necessary methods
}
