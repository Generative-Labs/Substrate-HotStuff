use std::{marker::PhantomData, ops::Add};
use finality_grandpa::voter_set::VoterSet;
use parity_scale_codec::{Decode, Encode};

use parking_lot::MappedMutexGuard;
use sc_consensus::shared_data::{SharedData, SharedDataLocked};
use sp_consensus_grandpa::{AuthorityList, AuthorityId};

use std::fmt::Debug;

/// A shared authority set.
pub struct SharedAuthoritySet<H, N> {
	inner: SharedData<AuthoritySet<H, N>>,
}

impl<H: Eq, N> SharedAuthoritySet<H, N>
where
	N: Add<Output = N> + Ord + Clone + Debug,
	H: Clone + Debug,
{
	/// Clone the inner `AuthoritySetChanges`.
	pub fn authority_set_changes(&self) -> AuthoritySetChanges<N> {
		self.inner().authority_set_changes.clone()
	}

	/// Get the current authorities and their weights (for the current set ID).
	pub fn current_authorities(&self) -> VoterSet<AuthorityId> {
		VoterSet::new(self.inner().current_authorities.iter().cloned()).expect(
			"current_authorities is non-empty and weights are non-zero; \
			 constructor and all mutating operations on `AuthoritySet` ensure this; \
			 qed.",
		)
	}
}

impl<H, N> SharedAuthoritySet<H, N> {
	/// Returns access to the [`AuthoritySet`].
	// pub(crate) fn inner(&self) -> MappedMutexGuard<AuthoritySet<H, N>> {
	// 	self.inner.shared_data()
	// }

	pub fn inner(&self) -> MappedMutexGuard<AuthoritySet<H, N>> {
		self.inner.shared_data()
	}

	/// Returns access to the [`AuthoritySet`] and locks it.
	///
	/// For more information see [`SharedDataLocked`].
	pub(crate) fn inner_locked(&self) -> SharedDataLocked<AuthoritySet<H, N>> {
		self.inner.shared_data_locked()
	}
}

/// Tracks historical authority set changes. We store the block numbers for the last block
/// of each authority set, once they have been finalized. These blocks are guaranteed to
/// have a justification unless they were triggered by a forced change.
#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct AuthoritySetChanges<N>(Vec<(u64, N)>);


/// A set of authorities.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritySet<H, N> {
    _phantom: PhantomData<H>,
	pub current_authorities: AuthorityList,
	pub(crate) authority_set_changes: AuthoritySetChanges<N>,
}

impl<H, N> From<AuthoritySet<H, N>> for SharedAuthoritySet<H, N> {   
	 fn from(set: AuthoritySet<H, N>) -> Self {       
		SharedAuthoritySet {
			inner: SharedData::new(set)
		}
	}
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


	/// Create a new authority set.
	pub(crate) fn new(
		authorities: AuthorityList,
		authority_set_changes: AuthoritySetChanges<N>,
	) -> Option<Self> {
		// if Self::invalid_authority_list(&authorities) {
		// 	return None
		// }

		// Some(AuthoritySet {
		// 	_phantom: PhantomData<H>,
		// 	current_authorities: authorities,
		// 	authority_set_changes,
		// })

		// TODO
		return None
	}

}
