
use crate::HOTSTUFF_ENGINE_ID;
use codec::{Codec, Encode};
use sp_consensus_slots::Slot;
use sp_runtime::generic::DigestItem;

/// A digest item which is usable with hotstuff consensus.
pub trait CompatibleDigestItem<Signature>: Sized {
	/// Construct a digest item which contains a signature on the hash.
	fn hotstuff_seal(signature: Signature) -> Self;

	/// If this item is an hotstuff seal, return the signature.
	fn as_hotstuff_seal(&self) -> Option<Signature>;

	/// Construct a digest item which contains the slot number
	fn hotstuff_pre_digest(slot: Slot) -> Self;

	/// If this item is an hotstuff pre-digest, return the slot number
	fn as_hotstuff_pre_digest(&self) -> Option<Slot>;
}

impl<Signature> CompatibleDigestItem<Signature> for DigestItem
where
	Signature: Codec,
{
	fn hotstuff_seal(signature: Signature) -> Self {
		DigestItem::Seal(HOTSTUFF_ENGINE_ID, signature.encode())
	}

	fn as_hotstuff_seal(&self) -> Option<Signature> {
		self.seal_try_to(&HOTSTUFF_ENGINE_ID)
	}

	fn hotstuff_pre_digest(slot: Slot) -> Self {
		DigestItem::PreRuntime(HOTSTUFF_ENGINE_ID, slot.encode())
	}

	fn as_hotstuff_pre_digest(&self) -> Option<Slot> {
		self.pre_runtime_try_to(&HOTSTUFF_ENGINE_ID)
	}
}
