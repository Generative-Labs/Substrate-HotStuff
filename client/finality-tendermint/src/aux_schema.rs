//! Schema for stuff in the aux-db.

use std::fmt::Debug;

use finality_tendermint::persistent::State as State;
use log::{info, warn};
use parity_scale_codec::{Decode, Encode};

use fork_tree::ForkTree;
use sc_client_api::backend::AuxStore;
use sp_blockchain::{Error as ClientError, Result as ClientResult};
use sp_finality_tendermint::{AuthorityList, SetId, RoundNumber};
use sp_runtime::traits::{Block as BlockT, NumberFor};

use crate::{
	authorities::{AuthoritySet, SharedAuthoritySet},
	environment::{SharedVoterSetState, VoterSetState},
	NewAuthoritySet, justification::TendermintJustification,
};

const VERSION_KEY: &[u8] = b"tendermint_schema_version";
const SET_STATE_KEY: &[u8] = b"tendermint_completed_round";
const CONCLUDED_ROUNDS: &[u8] = b"tendermint_concluded_rounds";
const AUTHORITY_SET_KEY: &[u8] = b"tendermint_voters";
const BEST_JUSTIFICATION: &[u8] = b"tendermint_best_justification";

const CURRENT_VERSION: u32 = 3;

pub(crate) fn load_decode<B: AuxStore, T: Decode>(
	backend: &B,
	key: &[u8],
) -> ClientResult<Option<T>> {
	match backend.get_aux(key)? {
		None => Ok(None),
		Some(t) => T::decode(&mut &t[..])
			.map_err(|e| ClientError::Backend(format!("TDMT DB is corrupted: {}", e)))
			.map(Some),
	}
}
/// Persistent data kept between runs.
pub(crate) struct PersistentData<Block: BlockT> {
	pub(crate) authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
	pub(crate) set_state: SharedVoterSetState<Block>,
}
/// Load or initialize persistent data from backend.
pub(crate) fn load_persistent<Block: BlockT, B, G>(
	backend: &B,
	genesis_hash: Block::Hash,
	genesis_number: NumberFor<Block>,
	genesis_authorities: G,
) -> ClientResult<PersistentData<Block>>
where
	B: AuxStore,
	G: FnOnce() -> ClientResult<AuthorityList>,
{
	let version: Option<u32> = load_decode(backend, VERSION_KEY)?;

	let make_genesis_view = move || State::genesis((genesis_hash, genesis_number));

	match version {
		Some(3) | None => {
			if let Some(set) = load_decode::<_, AuthoritySet<Block::Hash, NumberFor<Block>>>(
				backend,
				AUTHORITY_SET_KEY,
			)? {
				let set_state =
					match load_decode::<_, VoterSetState<Block>>(backend, SET_STATE_KEY)? {
						Some(state) => state,
						None => {
							let state = make_genesis_view();
							let base = state.finalized
							.expect("state is for completed round; completed rounds must have a prevote ghost; qed.");

							VoterSetState::live(set.set_id, &set, base)
						},
					};

				return Ok(PersistentData {
					authority_set: set.into(),
					set_state: set_state.into(),
				});
			}
		},
		Some(other) => {
			return Err(ClientError::Backend(format!(
				"Unsupported TDMT DB version: {:?}",
				other
			)))
		},
        _ => {
			return Err(ClientError::Backend(format!(
				"Unsupported TDMT DB version: None",
			)))
        }
	}

	// genesis.
	info!(target: "afp", "👴 Loading TDMT authority set \
		from genesis on what appears to be first startup.");

	let genesis_authorities = genesis_authorities()?;
	let genesis_set = AuthoritySet::genesis(genesis_authorities)
		.expect("genesis authorities is non-empty; all weights are non-zero; qed.");
	let state = make_genesis_view();
	let base = state
		.finalized
		.expect("state is for completed round; completed rounds must have a prevote ghost; qed.");

	let genesis_state = VoterSetState::live(0, &genesis_set, base);

	backend.insert_aux(
		&[
			(AUTHORITY_SET_KEY, genesis_set.encode().as_slice()),
			(SET_STATE_KEY, genesis_state.encode().as_slice()),
		],
		&[],
	)?;

	Ok(PersistentData { authority_set: genesis_set.into(), set_state: genesis_state.into() })
}

/// Update the authority set on disk after a change.
///
/// If there has just been a handoff, pass a `new_set` parameter that describes the
/// handoff. `set` in all cases should reflect the current authority set, with all
/// changes and handoffs applied.
pub(crate) fn update_authority_set<Block: BlockT, F, R>(
	set: &AuthoritySet<Block::Hash, NumberFor<Block>>,
	new_set: Option<&NewAuthoritySet<Block::Hash, NumberFor<Block>>>,
	write_aux: F,
) -> R
where
	F: FnOnce(&[(&'static [u8], &[u8])]) -> R,
{
	// write new authority set state to disk.
	let encoded_set = set.encode();

	if let Some(new_set) = new_set {
		// we also overwrite the "last completed round" entry with a blank slate
		// because from the perspective of the finality gadget, the chain has
		// reset.
		let set_state = VoterSetState::<Block>::live(
			new_set.set_id,
			&set,
			(new_set.canon_hash, new_set.canon_number),
		);
		let encoded = set_state.encode();

		write_aux(&[(AUTHORITY_SET_KEY, &encoded_set[..]), (SET_STATE_KEY, &encoded[..])])
	} else {
		write_aux(&[(AUTHORITY_SET_KEY, &encoded_set[..])])
	}
}

/// Update the justification for the latest finalized block on-disk.
///
/// We always keep around the justification for the best finalized block and overwrite it
/// as we finalize new blocks, this makes sure that we don't store useless justifications
/// but can always prove finality of the latest block.
pub(crate) fn update_best_justification<Block: BlockT, F, R>(
	justification: &TendermintJustification<Block>,
	write_aux: F,
) -> R
where
	F: FnOnce(&[(&'static [u8], &[u8])]) -> R,
{
	let encoded_justification = justification.encode();
	write_aux(&[(BEST_JUSTIFICATION, &encoded_justification[..])])
}

/// Write voter set state.
pub(crate) fn write_voter_set_state<Block: BlockT, B: AuxStore>(
	backend: &B,
	state: &VoterSetState<Block>,
) -> ClientResult<()> {
	backend.insert_aux(&[(SET_STATE_KEY, state.encode().as_slice())], &[])
}
