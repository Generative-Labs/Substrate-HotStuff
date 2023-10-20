use codec::{Decode, Encode, Error, Input};
use scale_info::TypeInfo;
use sp_std::{cmp, prelude::*};

use crate::{Payload, ValidatorSetId};

/// A commitment signed by Hotstuff validators as part of Hotstuff protocol.
///
/// The commitment contains a [payload](Commitment::payload) extracted from the finalized block at
/// height [block_number](Commitment::block_number).
/// Hotstuff validators collect signatures on commitments and a stream of such signed commitments
/// (see [SignedCommitment]) forms the Hotstuff protocol.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct Commitment<TBlockNumber> {
	///  A collection of payloads to be signed, see [`Payload`] for details.
	///
	/// One of the payloads should be some form of cumulative representation of the chain (think
	/// MMR root hash). Additionally one of the payloads should also contain some details that
	/// allow the light client to verify next validator set. The protocol does not enforce any
	/// particular format of this data, nor how often it should be present in commitments, however
	/// the light client has to be provided with full validator set whenever it performs the
	/// transition (i.e. importing first block with
	/// [validator_set_id](Commitment::validator_set_id) incremented).
	pub payload: Payload,

	/// Finalized block number this commitment is for.
	///
	/// Hotstuff validators agree on a block they create a commitment for and start collecting
	/// signatures. This process is called a round.
	/// There might be multiple rounds in progress (depending on the block choice rule), however
	/// since the payload is supposed to be cumulative, it is not required to import all
	/// commitments.
	/// Hotstuff light client is expected to import at least one commitment per epoch,
	/// but is free to import as many as it requires.
	pub block_number: TBlockNumber,

	/// Hotstuff validator set supposed to sign this commitment.
	///
	/// Validator set is changing once per epoch. The Light Client must be provided by details
	/// about the validator set whenever it's importing first commitment with a new
	/// `validator_set_id`. Validator set data MUST be verifiable, for instance using
	/// [payload](Commitment::payload) information.
	pub validator_set_id: ValidatorSetId,
}