// #![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]

use core::num::NonZeroUsize;

#[cfg(not(feature = "std"))]
#[macro_use]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(not(feature = "std"))]
mod std {
    pub use core::{cmp, hash, iter, mem, num, ops};

    pub mod vec {
        pub use alloc::vec::Vec;
    }

    pub mod collections {
        pub use alloc::collections::{
            btree_map::{self, BTreeMap},
            btree_set::{self, BTreeSet},
        };
    }

    pub mod fmt {
        pub use core::fmt::{Display, Formatter, Result};

        pub trait Debug {}
        impl<T> Debug for T {}
    }
}

/// Arithmetic necessary for a block number.
pub trait BlockNumberOps:
    std::fmt::Debug
    + std::cmp::Ord
    + std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + num::One
    + num::Zero
    + num::AsPrimitive<usize>
{
}

impl<T> BlockNumberOps for T
where
    T: std::fmt::Debug,
    T: std::cmp::Ord,
    T: std::ops::Add<Output = Self>,
    T: std::ops::Sub<Output = Self>,
    T: num::One,
    T: num::Zero,
    T: num::AsPrimitive<usize>,
{
}

use crate::std::{vec::Vec};

/// Error for Tendermint
#[derive(Clone, PartialEq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
pub enum Error {}

#[cfg(feature = "std")]
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            _ => write!(f, "not implemented"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

// #[cfg(feature = "std")]
pub mod messages;

#[cfg(feature = "std")]
pub mod environment;

#[cfg(feature = "std")]
pub mod voter;

// #[cfg(feature = "std")]
// mod rpc;

#[cfg(all(test, feature = "std"))]
pub(crate) mod testing;

pub mod persistent {
    #[cfg(feature = "derive-codec")]
    use parity_scale_codec::{Decode, Encode};
    

    /// State of the round.
    #[derive(PartialEq, Clone)]
    #[cfg_attr(any(feature = "std", test), derive(Debug))]
    #[cfg_attr(feature = "derive-codec", derive(Encode, Decode, scale_info::TypeInfo))]
    pub struct State<H, N> {
        /// The finalized block.
        pub finalized: Option<(H, N)>,
        /// Whether the view is completable.
        pub completable: bool,
    }

    impl<H: Clone, N: Clone> State<N, H> {
        /// Genesis state.
        pub fn genesis(genesis: (N, H)) -> Self {
            State {
                finalized: Some(genesis.clone()),
                completable: true,
            }
        }
    }
}

/// A set of nodes valid to vote.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
pub struct VoterSet<Id: Eq + Ord> {
    /// Voter's Id, with the same order as the vector in genesis block (or the following).
    voters: Vec<Id>,
    /// The required threshold number for supermajority.
    /// Normally, it's > 2/3.
    threshold: usize,
}

impl<Id: Eq + Ord + Clone> VoterSet<Id> {
    pub fn new(voters: Vec<Id>) -> Option<Self> {
        if voters.is_empty() {
            None
        } else {
            let len = voters.len() - (voters.len() - 1) / 3;
            Some(Self {
                voters,
                threshold: len,
            })
        }
    }

    pub fn add(&mut self, id: Id) {
        self.voters.push(id);
    }

    pub fn remove(&mut self, id: &Id) {
        self.voters.retain(|x| x != id);
    }

    pub fn is_empty(&self) -> bool {
        self.voters.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.voters.len() >= self.threshold
    }

    pub fn is_member(&self, id: &Id) -> bool {
        self.voters.contains(id)
    }

    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Get the size of the set.
    pub fn len(&self) -> NonZeroUsize {
        unsafe {
            // SAFETY: By VoterSet::new()
            NonZeroUsize::new_unchecked(self.voters.len())
        }
    }

    /// Get the nth voter in the set, if any.
    ///
    /// Returns `None` if `n >= len`.
    pub fn nth(&self, n: usize) -> Option<&Id> {
        self.voters.get(n)
    }

    /// Get a ref to voters.
    pub fn voters(&self) -> &[Id] {
        &self.voters
    }

    /// Get leader Id.
    pub fn get_proposer(&self, round: u64) -> Id {
        self.voters
            .get(round as usize % self.voters.len())
            .cloned()
            .unwrap()
    }

    /// Whether the set contains a voter with the given ID.
    pub fn contains(&self, id: &Id) -> bool {
        self.voters.contains(id)
    }

    /// Get an iterator over the voters in the set, as given by
    /// the associated total order.
    pub fn iter(&self) -> impl Iterator<Item = &Id> {
        self.voters.iter()
    }

    /// Get the voter info for the voter with the given ID, if any.
    pub fn get(&self, id: &Id) -> Option<&Id> {
        if let Some(pos) = self.voters.iter().position(|i| id == i) {
            self.voters.get(pos)
        } else {
            None
        }
    }
}
