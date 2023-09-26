pub use sp_core::H256;
// use sp_consensus::Proposer;

use sp_core::crypto::Pair;

pub type AuthorityId<P> = <P as Pair>::Public;
