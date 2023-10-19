use sp_consensus_hotstuff::AuthorityId;

// define some primitives used in hotstuff
pub type ViewNumber = u64;

// TODO The `AuthorityId` in this context should be reference instead of value?
#[derive(Debug, PartialEq, Eq)]
pub enum HotstuffError {
	// Receive more then one vote from the same authority.
	AuthorityReuse(AuthorityId),

	// The QC without a quorum
	InsufficientQuorum,

	// Get invalid signature from a authority.
	InvalidSignature(AuthorityId),

	NullSignature,

	UnknownAuthority(AuthorityId),

	// The voter is not in authorities.
	NotAuthority,

	WrongProposer,

	// can't find the parent of this proposal.
	// TODO add proposal info.
	ProposalNoParent,

	// Vote from old view.
	ExpiredVote,

	InvalidTC,

	FinalizeBlock(String),

	SaveProposal(String),

	// Error generate by node client.
	ClientError(String),

	Other(String),
}
