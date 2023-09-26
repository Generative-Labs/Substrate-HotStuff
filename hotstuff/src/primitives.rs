use sp_consensus_hotstuff::AuthorityId;

// define some primitives used in hotstuff
pub type ViewNumber = u64;

#[derive(Debug)]
pub enum HotstuffError {
	// Receive more then one vote from the same authority.
	AuthorityReuse(AuthorityId),

	// The QC without a quorum
	QCRequiresQuorum,

	// Get invalid signature from a authority
	InvalidSignature(AuthorityId),

	Other(String),
}
