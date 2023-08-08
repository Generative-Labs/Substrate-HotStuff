#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use serde::Serialize;

use codec::{Codec, Decode, Encode};
// use scale_info::TypeInfo;
use sp_core::crypto::Pair;


#[cfg(feature = "std")]
use sp_keystore::{Keystore, KeystorePtr};
// use sp_runtime::{traits::NumberFor, ConsensusEngineId, RuntimeDebug};
use sp_runtime::{ConsensusEngineId, RuntimeDebug};
// use sp_std::{borrow::Cow, vec::Vec};
use sp_std::{vec::Vec};

#[cfg(feature = "std")]
use log::debug;

use finality_tendermint::messages;

use sp_core::crypto::KeyTypeId;

/// Key type for Tendermint module
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"tdmt");

/// Identify of a Tendermint authority.
pub type AuthorityId = app::Public;


mod app {
	// use sp_core::crypto;

	// use sp_application_crypto::{app_crypto, ed25519};
	use sp_application_crypto::{app_crypto, ed25519, KeyTypeId};

	app_crypto!(ed25519, KeyTypeId(*b"tdmt"));
}

sp_application_crypto::with_pair! {
	/// The Tendermint crypto scheme defined via the keypair type.
	pub type AuthorityPair = app::Pair;
}

/// Identify of a Tendermint authority.
// pub type AuthorityId = app::Public;

/// Signature for a Tendermint authority.
pub type AuthoritySignature = app::Signature;

/// The `ConsensusEngineId` of PBFT.
pub const TDMT_ENGINE_ID: ConsensusEngineId = *b"TDMT";


/// The storage key for the current set of weighted PBFT authorities.
/// The value stored is an encoded VersionedAuthorityList.
pub const TDMT_AUTHORITIES_KEY: &'static [u8] = b":tendermint_authorities";

/// The index of an authority.
pub type AuthorityIndex = u64;

/// The monotonic identifier of a PBFT set of authorities.
pub type SetId = u64;

/// The round indicator.
pub type RoundNumber = u64;

/// A list of Grandpa authorities with associated weights.
pub type AuthorityList = Vec<AuthorityId>;

#[cfg_attr(feature = "std", derive(Serialize))]
#[derive(Clone, Eq, PartialEq, Encode, Decode, RuntimeDebug)]
pub struct ScheduledChange<N> {
	/// The new authorities after the change, along with their respective weights.
	pub next_authorities: AuthorityList,
	/// The number of blocks to delay.
	pub delay: N,
}

/// An consensus log item for GRANDPA.
#[cfg_attr(feature = "std", derive(Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone, RuntimeDebug)]
pub enum ConsensusLog<N: Codec> {
	/// Schedule an authority set change.
	///
	/// The earliest digest of this type in a single block will be respected,
	/// provided that there is no `ForcedChange` digest. If there is, then the
	/// `ForcedChange` will take precedence.
	///
	/// No change should be scheduled if one is already and the delay has not
	/// passed completely.
	///
	/// This should be a pure function: i.e. as long as the runtime can interpret
	/// the digest type it should return the same result regardless of the current
	/// state.
	#[codec(index = 1)]
	ScheduledChange(ScheduledChange<N>),
	/// Force an authority set change.
	///
	/// Forced changes are applied after a delay of _imported_ blocks,
	/// while pending changes are applied after a delay of _finalized_ blocks.
	///
	/// The earliest digest of this type in a single block will be respected,
	/// with others ignored.
	///
	/// No change should be scheduled if one is already and the delay has not
	/// passed completely.
	///
	/// This should be a pure function: i.e. as long as the runtime can interpret
	/// the digest type it should return the same result regardless of the current
	/// state.
	#[codec(index = 2)]
	ForcedChange(N, ScheduledChange<N>),
	/// Note that the authority with given index is disabled until the next change.
	#[codec(index = 3)]
	OnDisabled(AuthorityIndex),
	/// A signal to pause the current authority set after the given delay.
	/// After finalizing the block at _delay_ the authorities should stop voting.
	#[codec(index = 4)]
	Pause(N),
	/// A signal to resume the current authority set after the given delay.
	/// After authoring the block at _delay_ the authorities should resume voting.
	#[codec(index = 5)]
	Resume(N),
}

impl<N: Codec> ConsensusLog<N> {
	/// Try to cast the log entry as a contained signal.
	pub fn try_into_change(self) -> Option<ScheduledChange<N>> {
		match self {
			ConsensusLog::ScheduledChange(change) => Some(change),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained forced signal.
	pub fn try_into_forced_change(self) -> Option<(N, ScheduledChange<N>)> {
		match self {
			ConsensusLog::ForcedChange(median, change) => Some((median, change)),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained pause signal.
	pub fn try_into_pause(self) -> Option<N> {
		match self {
			ConsensusLog::Pause(delay) => Some(delay),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained resume signal.
	pub fn try_into_resume(self) -> Option<N> {
		match self {
			ConsensusLog::Resume(delay) => Some(delay),
			_ => None,
		}
	}
}

/// Encode round message localized to a given round and set id.
pub fn localized_payload<E: Encode>(round: u64, set_id: SetId, message: &E) -> Vec<u8> {
	let mut buf = Vec::new();
	localized_payload_with_buffer(round, set_id, message, &mut buf);
	buf
}

/// Encode round message localized to a given round and set id using the given
/// buffer. The given buffer will be cleared and the resulting encoded payload
/// will always be written to the start of the buffer.
pub fn localized_payload_with_buffer<E: Encode>(
	round: u64,
	set_id: SetId,
	message: &E,
	buf: &mut Vec<u8>,
) {
	buf.clear();
	(message, round, set_id).encode_to(buf)
}

/// Check a message signature by encoding the message as a localized payload and
/// verifying the provided signature using the expected authority id.
pub fn check_message_signature<H, N>(
	message: &messages::Message<H, N>,
	id: &AuthorityId,
	signature: &AuthoritySignature,
	round: u64,
	set_id: SetId,
) -> bool
where
	H: Encode,
	N: Encode,
{
	check_message_signature_with_buffer(message, id, signature, round, set_id, &mut Vec::new())
}

/// Check a message signature by encoding the message as a localized payload and
/// verifying the provided signature using the expected authority id.
/// The encoding necessary to verify the signature will be done using the given
/// buffer, the original content of the buffer will be cleared.
pub fn check_message_signature_with_buffer<H, N>(
	message: &messages::Message<H, N>,
	id: &AuthorityId,
	signature: &AuthoritySignature,
	round: u64,
	set_id: SetId,
	buf: &mut Vec<u8>,
) -> bool
where
	H: Encode,
	N: Encode,
{
	use sp_application_crypto::RuntimeAppPublic;

	localized_payload_with_buffer(round, set_id, message, buf);

	let valid = id.verify(&buf, signature);

	if !valid {
		#[cfg(feature = "std")]
		debug!(target: "afg", "Bad signature on message from {:?}", id);
	}

	valid
}


/// Localizes the message to the given set and round and signs the payload.
#[cfg(feature = "std")]
pub fn sign_message<H, N>(
	keystore: KeystorePtr,
	message: messages::Message<H, N>,
	public: AuthorityId,
	round: RoundNumber,
	set_id: SetId,
) -> Option<messages::SignedMessage<H, N, AuthoritySignature, AuthorityId>>
where
	H: Encode,
	N: Encode,
{

	use sp_application_crypto::KeyTypeId;
	use sp_application_crypto::CryptoTypeId;

    let crypto_id: CryptoTypeId = CryptoTypeId(*b"ed25");

	let keytype_id: KeyTypeId = KeyTypeId(*b"tdmt");

	let encoded = localized_payload(round, set_id, &message);

    // let public_ref: &[u8] = &public.into_inner();
	let public_ref =  public.as_ref();

	let signature = Keystore::sign_with(
		&keystore,
		keytype_id,
		crypto_id,
		public_ref,
		&encoded[..],
	)
	.ok()
	.flatten()?
	.try_into()
	.ok()?;

	Some(messages::SignedMessage { message, signature, id: public })
}

sp_api::decl_runtime_apis! {
	/// APIs for integrating the PBFT finality gadget into runtimes.
	/// This should be implemented on the runtime side.
	///
	/// This is primarily used for negotiating authority-set changes for the
	/// gadget. GRANDPA uses a signaling model of changing authority sets:
	/// changes should be signaled with a delay of N blocks, and then automatically
	/// applied in the runtime after those N blocks have passed.
	///
	/// The consensus protocol will coordinate the handoff externally.
	#[api_version(3)]
	pub trait TendermintApi {
		/// Get the current GRANDPA authorities and weights. This should not change except
		/// for when changes are scheduled and the corresponding delay has passed.
		///
		/// When called at block B, it will return the set of authorities that should be
		/// used to finalize descendants of this block (B+1, B+2, ...). The block B itself
		/// is finalized by the authorities from block B-1.
		fn tendermint_authorities() -> AuthorityList;
		/// Get current GRANDPA authority set id.
		fn current_set_id() -> SetId;
	}
}
