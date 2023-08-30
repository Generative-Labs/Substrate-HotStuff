#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode};

use sp_runtime::ConsensusEngineId;
use sp_std::vec::Vec;


// use sp_application_crypto::RuntimeAppPublic;

/// The log target to be used by client code.
pub const CLIENT_LOG_TARGET: &str = "hotstuff";
/// The log target to be used by runtime code.
pub const RUNTIME_LOG_TARGET: &str = "runtime::hotstuff";


/// Key type for HOTSTUFF module.
pub const HOTSTUFF_KEY_TYPE: sp_core::crypto::KeyTypeId = sp_core::crypto::KeyTypeId(*b"hots");

/// Authority set id starts with zero at HOTSTUFF pallet genesis.
pub const GENESIS_AUTHORITY_SET_ID: u64 = 0;


pub use sp_consensus_slots::{Slot, SlotDuration};

/// A typedef for validator set id.
pub type ValidatorSetId = u64;

mod app {
    use crate::HOTSTUFF_KEY_TYPE;

	use sp_application_crypto::{app_crypto, ed25519};
	app_crypto!(ed25519, HOTSTUFF_KEY_TYPE);
}


sp_application_crypto::with_pair! {
	/// The hotstuff crypto scheme defined via the keypair type.
	pub type AuthorityPair = app::Pair;
}


pub mod sr25519 {
	mod app_sr25519 {
        use crate::HOTSTUFF_KEY_TYPE;

		use sp_application_crypto::{app_crypto, sr25519};
		app_crypto!(sr25519, HOTSTUFF_KEY_TYPE);
	}

	sp_application_crypto::with_pair! {
		/// An Hotstuff authority keypair using S/R 25519 as its crypto.
		pub type AuthorityPair = app_sr25519::Pair;
	}

	/// An Hotstuff authority signature using S/R 25519 as its crypto.
	pub type AuthoritySignature = app_sr25519::Signature;

	/// An Hotstuff authority identifier using S/R 25519 as its crypto.
	pub type AuthorityId = app_sr25519::Public;
}

pub mod ed25519 {
	mod app_ed25519 {
        use crate::HOTSTUFF_KEY_TYPE;

		use sp_application_crypto::{app_crypto, ed25519};
		app_crypto!(ed25519, HOTSTUFF_KEY_TYPE);
	}

	sp_application_crypto::with_pair! {
		/// An Hotstuff authority keypair using Ed25519 as its crypto.
		pub type AuthorityPair = app_ed25519::Pair;
	}

	/// An Hotstuff authority signature using Ed25519 as its crypto.
	pub type AuthoritySignature = app_ed25519::Signature;

	/// An Hotstuff authority identifier using Ed25519 as its crypto.
	pub type AuthorityId = app_ed25519::Public;
}


// /// Identity of a Hotstuff authority.
pub type AuthorityId = app::Public;


/// Signature for a Hotstuff authority.
pub type AuthoritySignature = app::Signature;


/// The `ConsensusEngineId` of HOTSTUFF.
pub const HOTSTUFF_ENGINE_ID: ConsensusEngineId = *b"HOTS";


/// The storage key for the current set of weighted hotstuff authorities.
/// The value stored is an encoded VersionedAuthorityList.
pub const HOTSTUFF_AUTHORITIES_KEY: &[u8] = b":hotstuff_authorities";


/// The weight of an authority.
pub type AuthorityWeight = u64;

/// The index of an authority.
pub type AuthorityIndex = u64;

/// The monotonic identifier of a Hotstuff set of authorities.
pub type SetId = u64;

/// The round indicator.
pub type RoundNumber = u64;


/// A list of Hotstuff authorities with associated weights.
pub type AuthorityList = Vec<(AuthorityId, AuthorityWeight)>;


/// An consensus log item for Hotstuff.
#[derive(Decode, Encode)]
pub enum ConsensusLog<AuthorityId: Codec> {
	/// The authorities have changed.
	#[codec(index = 1)]
	AuthoritiesChange(Vec<AuthorityId>),
	/// Disable the authority with given index.
	#[codec(index = 2)]
	OnDisabled(AuthorityIndex),
}

sp_api::decl_runtime_apis! {
	/// API necessary for block authorship with hotstuff.
	pub trait HotstuffApi<AuthorityId: Codec> {
		// /// Returns the slot duration for hotstuff.
		// ///
		// /// Currently, only the value provided by this type at genesis will be used.
		// fn slot_duration() -> SlotDuration;

		/// Return the current set of authorities.
		fn authorities() -> Vec<AuthorityId>;
	}
}
