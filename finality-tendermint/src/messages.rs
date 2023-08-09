#[cfg(feature = "derive-codec")]
use parity_scale_codec::{Decode, Encode};
#[cfg(feature = "derive-codec")]
use scale_info::TypeInfo;

#[cfg(not(feature = "std"))]
use crate::std::vec::Vec;

/// A preprepare message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Proposal<N, D> {
    pub round: u64,
    pub target_height: N,
    pub target_hash: D,
    pub valid_round: Option<u64>,
}

/// A preprepare message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Prevote<N, D> {
    pub round: u64,
    pub target_height: N,
    pub target_hash: Option<D>,
}

/// A preprepare message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Precommit<N, D> {
    pub round: u64,
    pub target_height: N,
    pub target_hash: Option<D>,
}

#[derive(Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub enum Message<N, H> {
    Proposal(Proposal<N, H>),
    Prevote(Prevote<N, H>),
    Precommit(Precommit<N, H>),
}

impl<N: Clone, H: Clone> Message<N, H> {
    pub fn round(&self) -> u64 {
        match self {
            Message::Proposal(p) => p.round,
            Message::Prevote(p) => p.round,
            Message::Precommit(p) => p.round,
        }
    }

    pub fn target(&self) -> (Option<H>, N) {
        match self {
            Message::Proposal(p) => (Some(p.target_hash.clone()), p.target_height.clone()),
            Message::Prevote(p) => (p.target_hash.clone(), p.target_height.clone()),
            Message::Precommit(p) => (p.target_hash.clone(), p.target_height.clone()),
        }
    }
}

/// Signed Messages
#[derive(Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedMessage<N, H, Sig, Id> {
    pub id: Id,
    pub signature: Sig,
    pub message: Message<N, H>,
}

impl<N: Clone, H: Clone, Sig: Clone, Id: Clone> SignedMessage<N, H, Sig, Id> {
    pub fn round(&self) -> u64 {
        self.message.round()
    }

    pub fn target(&self) -> (Option<H>, N) {
        self.message.target()
    }
}

/// A signed commit message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedCommit<N, D, S, Id> {
    /// The commit message which has been signed.
    pub commit: Precommit<N, D>,
    /// The signature on the message.
    pub signature: S,
    /// The Id of the signer.
    pub id: Id,
}

impl<N, D, S, Id> From<SignedCommit<N, D, S, Id>> for SignedMessage<N, D, S, Id> {
    fn from(commit: SignedCommit<N, D, S, Id>) -> Self {
        SignedMessage {
            id: commit.id,
            signature: commit.signature,
            message: Message::Precommit(commit.commit),
        }
    }
}

/// Signed Messages
#[derive(PartialEq, Eq, Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct FinalizedCommit<N, D, Sig, Id> {
    /// The target block's hash.
    pub target_hash: D,
    /// The target block's number.
    pub target_number: N,
    /// Precommits for target block or any block after it that justify this commit.
    pub commits: Vec<SignedCommit<N, D, Sig, Id>>,
}

/// A commit message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Commit<N, D> {
    /// The view number.
    pub view: u64,
    pub seq: u64,
    /// The sequence number.
    pub target_number: N,
    /// The target block's hash.
    pub target_hash: D,
}

impl<N, D> Commit<N, D> {
    /// Create a new commit message.
    pub fn new(view: u64, seq: u64, target_number: N, target_hash: D) -> Self {
        Commit {
            view,
            seq,
            target_number,
            target_hash,
        }
    }
}

/// Authentication data for a set of many messages, currently a set of commit signatures
pub type MultiAuthData<S, Id> = Vec<(S, Id)>;

/// A commit message with compact representation of authenticationg data.
/// NOTE: Similar to `CompactCommit`
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct CompactCommit<D, N, S, Id> {
    /// The target block's hash.
    pub target_hash: D,
    /// The target block's number.
    pub target_number: N,
    /// Precommits for target block or any block after it that justify this commit.
    pub commits: Vec<Precommit<N, D>>,
    /// Authentication data for the commit.
    pub auth_data: MultiAuthData<S, Id>,
}

/// Convert from [`FinalizedCommit`] to [`CompactCommit`].
impl<D: Clone, N: Clone, S, Id> From<FinalizedCommit<N, D, S, Id>> for CompactCommit<D, N, S, Id> {
    fn from(commit: FinalizedCommit<N, D, S, Id>) -> Self {
        CompactCommit {
            target_hash: commit.target_hash,
            target_number: commit.target_number,
            commits: commit.commits.iter().map(|c| c.commit.clone()).collect(),
            auth_data: commit
                .commits
                .into_iter()
                .map(|c| (c.signature, c.id))
                .collect(),
        }
    }
}

/// Convert from [`CompactCommit`] to [`FinalizedCommit`].
impl<D, N, S, Id> From<CompactCommit<D, N, S, Id>> for FinalizedCommit<N, D, S, Id> {
    fn from(commit: CompactCommit<D, N, S, Id>) -> Self {
        FinalizedCommit {
            target_hash: commit.target_hash,
            target_number: commit.target_number,
            commits: commit
                .commits
                .into_iter()
                .zip(commit.auth_data.into_iter())
                .map(|(c, (s, id))| SignedCommit {
                    commit: c,
                    signature: s,
                    id,
                })
                .collect(),
        }
    }
}

/// Struct returned from `validate_commit` function with information
/// about the validation result.
///
/// Stale code.
pub struct CommitValidationResult<H, N> {
    target: Option<(H, N)>,
    num_commits: usize,
    num_duplicated_commits: usize,
    num_invalid_voters: usize,
}

impl<H, N> CommitValidationResult<H, N> {
    pub fn target(&self) -> Option<&(H, N)> {
        self.target.as_ref()
    }

    /// Returns the number of commits in the commit.
    pub fn num_commits(&self) -> usize {
        self.num_commits
    }

    /// Returns the number of duplicate commits in the commit.
    pub fn num_duplicated_commits(&self) -> usize {
        self.num_duplicated_commits
    }

    /// Returns the number of invalid voters in the commit.
    pub fn num_invalid_voters(&self) -> usize {
        self.num_invalid_voters
    }
}

impl<H, N> Default for CommitValidationResult<H, N> {
    fn default() -> Self {
        CommitValidationResult {
            target: None,
            num_commits: 0,
            num_duplicated_commits: 0,
            num_invalid_voters: 0,
        }
    }
}

/// Communication between nodes that is not round-localized.
#[cfg(feature = "std")]
#[cfg_attr(any(test, feature = "test-helpers"), derive(Clone))]
#[derive(Debug)]
pub enum GlobalMessageIn<D, N, S, Id> {
    /// A commit message.
    Commit(
        u64,
        CompactCommit<D, N, S, Id>,
        Callback<CommitProcessingOutcome>,
    ),
    /// A catch up message.
    // CatchUp(CatchUp<N, D, S, Id>, Callback<CatchUpProcessingOutcome>),
    /// multicast <view + 1, latest stable checkpoint, C: a set of pairs with the sequence number
    /// A workaround for test network.
    Empty,
}

#[cfg(feature = "std")]
impl<D, N, S, Id> PartialEq for GlobalMessageIn<D, N, S, Id> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            _ => false,
        }
    }
}

#[cfg(feature = "std")]
impl<D, N, S, Id> Unpin for GlobalMessageIn<D, N, S, Id> {}

/// Communication between nodes that is not round-localized.
#[cfg(feature = "std")]
#[cfg_attr(any(test, feature = "test-helpers"), derive(Clone))]
#[derive(Debug)]
pub enum GlobalMessageOut<D, N, S, Id> {
    /// A commit message.
    Commit(
        u64,
        FinalizedCommit<N, D, S, Id>,
        // Callback<CommitProcessingOutcome>
    ),
    /// multicast <view + 1, latest stable checkpoint, C: a set of pairs with the sequence number
    /// and digest of each checkpoint, P, Q, i>
    Empty,
}

/// Callback used to pass information about the outcome of importing a given
/// message (e.g. vote, commit, catch up). Useful to propagate data to the
/// network after making sure the import is successful.
#[cfg(feature = "std")]
pub enum Callback<O> {
    /// Default value.
    Blank,
    /// Callback to execute given a processing outcome.
    Work(Box<dyn FnMut(O) + Send>),
}

#[cfg(feature = "std")]
impl<O> std::fmt::Debug for Callback<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blank => write!(f, "Blank"),
            Self::Work(_arg0) => write!(f, "Work"),
        }
    }
}

#[cfg(feature = "std")]
#[cfg(any(test, feature = "test-helpers"))]
impl<O> Clone for Callback<O> {
    fn clone(&self) -> Self {
        Callback::Blank
    }
}

#[cfg(feature = "std")]
impl<O> Callback<O> {
    /// Do the work associated with the callback, if any.
    pub fn run(&mut self, o: O) {
        match self {
            Callback::Blank => {}
            Callback::Work(cb) => cb(o),
        }
    }
}

/// The outcome of processing a commit.
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitProcessingOutcome {
    /// It was beneficial to process this commit.
    Good(GoodCommit),
    /// It wasn't beneficial to process this commit. We wasted resources.
    Bad(BadCommit),
}

#[cfg(feature = "std")]
#[cfg(any(test, feature = "test-helpers"))]
impl CommitProcessingOutcome {
    /// Returns a `Good` instance of commit processing outcome's opaque type. Useful for testing.
    pub fn good() -> CommitProcessingOutcome {
        CommitProcessingOutcome::Good(GoodCommit::new())
    }

    /// Returns a `Bad` instance of commit processing outcome's opaque type. Useful for testing.
    pub fn bad() -> CommitProcessingOutcome {
        CommitProcessingOutcome::Bad(CommitValidationResult::<(), ()>::default().into())
    }
}

/// The result of processing for a good commit.
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodCommit {}

#[cfg(feature = "std")]
impl GoodCommit {
    pub(crate) fn new() -> Self {
        GoodCommit {}
    }
}

/// The result of processing for a bad commit
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCommit {
    num_commits: usize,
    num_duplicated_commits: usize,
    num_invalid_voters: usize,
}

#[cfg(feature = "std")]
impl BadCommit {
    /// Get the number of precommits
    pub fn num_commits(&self) -> usize {
        self.num_commits
    }

    /// Get the number of duplicated precommits
    pub fn num_duplicated(&self) -> usize {
        self.num_duplicated_commits
    }

    /// Get the number of invalid voters in the precommits
    pub fn num_invalid_voters(&self) -> usize {
        self.num_invalid_voters
    }
}

#[cfg(feature = "std")]
impl<H, N> From<CommitValidationResult<H, N>> for BadCommit {
    fn from(r: CommitValidationResult<H, N>) -> Self {
        BadCommit {
            num_commits: r.num_commits,
            num_duplicated_commits: r.num_duplicated_commits,
            num_invalid_voters: r.num_invalid_voters,
        }
    }
}
