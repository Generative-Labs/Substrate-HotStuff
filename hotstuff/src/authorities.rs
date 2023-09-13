use std::marker::PhantomData;

use sc_consensus::shared_data::SharedData;
use sp_consensus_grandpa::AuthorityList;

use std::fmt::Debug;

/// A shared authority set.
pub struct SharedAuthoritySet<H, N> {
	inner: SharedData<AuthoritySet<H, N>>,
}


/// A set of authorities.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritySet<H, N> {
    _phantom: PhantomData<H>,
    _phantomN: PhantomData<N>,
}

impl<H, N> AuthoritySet<H, N>
where
	H: PartialEq,
	N: Ord + Clone,
{
	// authority sets must be non-empty and all weights must be greater than 0
	fn invalid_authority_list(authorities: &AuthorityList) -> bool {
		authorities.is_empty() || authorities.iter().any(|(_, w)| *w == 0)
	}

	/// Get a genesis set with given authorities.
	pub(crate) fn genesis(initial: AuthorityList) -> Option<Self> {
        // TODO
        return None

		// Some(AuthoritySet {
		// 	current_authorities: initial,
		// 	set_id: 0,
		// 	pending_standard_changes: ForkTree::new(),
		// 	pending_forced_changes: Vec::new(),
		// 	authority_set_changes: AuthoritySetChanges::empty(),
		// })
	}

}
