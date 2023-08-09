use futures::{Future, Sink, Stream};
use tracing::Value;

use crate::{
    messages::{FinalizedCommit, GlobalMessageIn, GlobalMessageOut, Message, SignedMessage},
    BlockNumberOps, Error,
};

/// Necessary environment for a voter.
///
/// This encapsulates the database and networking layers of the chain.
pub trait Environment {
    /// Associated timer type for the environment. See also [`Self::round_data`] and
    /// [`Self::round_commit_timer`].
    type Timer: Future<Output = Result<(), Self::Error>> + Unpin;
    /// The associated Id for the Environment.
    type Id: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug;
    /// The associated Signature type for the Environment.
    type Signature: Eq + Clone + core::fmt::Debug;
    /// Associated future type for the environment used when asynchronously computing the
    /// best chain to vote on. See also [`Self::best_chain_containing`].
    type BestChain: Future<Output = Result<Option<(Self::Number, Self::Hash)>, Self::Error>>
        + Send
        + Unpin;
    /// The input stream used to communicate with the outside world.
    type In: Stream<
            Item = Result<
                SignedMessage<Self::Number, Self::Hash, Self::Signature, Self::Id>,
                Self::Error,
            >,
        > + Unpin;
    /// The output stream used to communicate with the outside world.
    type Out: Sink<Message<Self::Number, Self::Hash>, Error = Self::Error> + Unpin;
    /// The associated Error type.
    type Error: From<Error> + ::std::error::Error;
    /// Hash type used in blockchain or digest.
    type Hash: Eq + Clone + core::fmt::Debug;
    /// The block number type.
    type Number: BlockNumberOps;
    /// The input stream used to communicate with the outside world.
    type GlobalIn: Stream<
            Item = Result<
                GlobalMessageIn<Self::Hash, Self::Number, Self::Signature, Self::Id>,
                Self::Error,
            >,
        > + Unpin;
    /// The output stream used to communicate with the outside world.
    type GlobalOut: Sink<
            GlobalMessageOut<Self::Hash, Self::Number, Self::Signature, Self::Id>,
            Error = Self::Error,
        > + Unpin;

    /// Get Voter data.
    fn init_voter(&self) -> VoterData<Self::Id>;

    /// Get round data.
    fn init_round(&self, view: u64) -> RoundData<Self::Id, Self::In, Self::Out>;

    /// propose
    fn propose(&self, round: u64, block: Self::Hash) -> Self::BestChain;

    /// Similar to `completed`
    // fn complete_f_commit(
    //     &self,
    //     view: u64,
    //     state: State<Self::Number, Self::Hash>,
    //     base: (Self::Number, Self::Hash),
    //     f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
    // ) -> Result<(), Self::Error>;

    /// Finalize a block.
    // TODO: maybe async?
    fn finalize_block(
        &self,
        view: u64,
        hash: Self::Hash,
        number: Self::Number,
        f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
    ) -> Result<(), Self::Error>;
}

/// Data necessary to create a voter.
pub struct VoterData<Id: Ord> {
    /// Local voter id.
    pub local_id: Id,
    // pub global_in: GlobalIn,
    // pub global_out: GlobalOut,
    // pub voters: VoterSet<Id>,
    // pub finalized_target: (N, D),
}

/// Data necessary to participate in a round.
pub struct RoundData<Id, Input, Output> {
    /// Local voter id
    pub local_id: Id,
    // Timer before prevotes can be cast. This should be Start + 2T
    // where T is the gossip time estimate.
    // pub prevote_timer: Timer,
    /// Timer before precommits can be cast. This should be Start + 4T
    // pub precommit_timer: Timer,
    /// Incoming messages.
    pub incoming: Input,
    /// Outgoing messages.
    pub outgoing: Output,
    // Output state log
    // pub log_sender: LogOutput,
}
