#![cfg_attr(not(feature = "std"), no_std)]
use codec::{Codec, Decode, Encode};
use sp_std::vec::Vec;
use sp_runtime::ConsensusEngineId;


/// The `ConsensusEngineId` of HotStuff.
pub const HOTSTUFF_ENGINE_ID: ConsensusEngineId = [b'h', b'o', b't', b's'];

pub mod sr25519 {
	mod app_sr25519 {
		use sp_application_crypto::{app_crypto, key_types::AURA, sr25519};
		app_crypto!(sr25519, AURA);
	}

	sp_application_crypto::with_pair! {
		/// An Aura authority keypair using S/R 25519 as its crypto.
		pub type AuthorityPair = app_sr25519::Pair;
	}

	/// An Aura authority signature using S/R 25519 as its crypto.
	pub type AuthoritySignature = app_sr25519::Signature;

	/// An Aura authority identifier using S/R 25519 as its crypto.
	pub type AuthorityId = app_sr25519::Public;
}


pub mod ed25519 {
	mod app_ed25519 {
		use sp_application_crypto::{app_crypto, ed25519, key_types::AURA};
		app_crypto!(ed25519, AURA);
	}

	sp_application_crypto::with_pair! {
		/// An Aura authority keypair using Ed25519 as its crypto.
		pub type AuthorityPair = app_ed25519::Pair;
	}

	/// An Aura authority signature using Ed25519 as its crypto.
	pub type AuthoritySignature = app_ed25519::Signature;

	/// An Aura authority identifier using Ed25519 as its crypto.
	pub type AuthorityId = app_ed25519::Public;
}


pub use sp_consensus_slots::{Slot, SlotDuration};

/// The index of an authority.
pub type AuthorityIndex = u32;

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
		/// Returns the slot duration for Hotstuff.
		// ///
		// /// Currently, only the value provided by this type at genesis will be used.
		// fn slot_duration() -> SlotDuration;

		/// Return the current set of authorities.
		fn authorities() -> Vec<AuthorityId>;
	}
}

