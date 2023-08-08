#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

pub use sp_finality_tendermint as fp_primitives;

use codec::{self as codec, Decode, Encode, MaxEncodedLen};
pub use fp_primitives::{AuthorityId, AuthorityList};
use fp_primitives::{ConsensusLog, ScheduledChange, SetId, TDMT_AUTHORITIES_KEY, TDMT_ENGINE_ID};
use sp_std::prelude::*;

use frame_support::{
	dispatch::DispatchResultWithPostInfo,
	pallet_prelude::Get,
	storage,
	traits::{KeyOwnerProofSystem, OneSessionHandler, StorageVersion},
	weights::{Pays, Weight},
	WeakBoundedVec,
};
use sp_runtime::{generic::DigestItem, traits::Zero, DispatchResult};
use sp_session::{GetSessionNumber, GetValidatorCount};
use sp_staking::SessionIndex;


use scale_info::TypeInfo;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		// /// The event type of this module.
		// type Event: From<Event>
		// 	+ Into<<Self as frame_system::Config>::Event>
		// 	+ IsType<<Self as frame_system::Config>::Event>;
		//
		// /// The function call.
		// type Call: From<Call<Self>>;

		/// Max Authorities in use
		#[pallet::constant]
		type MaxAuthorities: Get<u32>;
	}

	/// The number of changes (both in terms of keys and underlying economic responsibilities)
	/// in the "set" of Grandpa validators from genesis.
	#[pallet::storage]
	#[pallet::getter(fn current_set_id)]
	pub(super) type CurrentSetId<T: Config> = StorageValue<_, SetId, ValueQuery>;

	// #[cfg_attr(feature = "std", derive(Default))]
	// #[pallet::genesis_config]
	// pub struct GenesisConfig {
	// 	pub authorities: AuthorityList,
	// }

	// #[pallet::genesis_build]
	// impl<T: Config> <pallet::GenesisConfig as GenesisBuild<T>>::build for GenesisConfig {
	// 	fn build(&self) {
	// 		CurrentSetId::<T>::put(fp_primitives::SetId::default());
	// 		Pallet::<T>::initialize(&self.authorities)
	// 	}
	// }
}

impl<T: Config> Pallet<T> {
	/// Get the current set of authorities, along with their respective weights.
	pub fn tendermint_authorities() -> AuthorityList {
		storage::unhashed::get_or_default::<AuthorityList>(TDMT_AUTHORITIES_KEY).into()
	}

	/// Set the current set of authorities, along with their respective weights.
	fn set_tendermint_authorities(authorities: &AuthorityList) {
		storage::unhashed::put(TDMT_AUTHORITIES_KEY, &authorities);
	}

	// Perform module initialization, abstracted so that it can be called either through genesis
	// config builder or through `on_genesis_session`.
	fn initialize(authorities: &AuthorityList) {
		if !authorities.is_empty() {
			assert!(Self::tendermint_authorities().is_empty(), "Authorities are already initialized!");
			Self::set_tendermint_authorities(authorities);
		}

		// NOTE: initialize first session of first set. this is necessary for
		// the genesis set and session since we only update the set -> session
		// mapping whenever a new session starts, i.e. through `on_new_session`.
		// SetIdSession::<T>::insert(0, 0);
	}
}

impl<T: Config> sp_runtime::BoundToRuntimeAppPublic for Pallet<T> {
	type Public = AuthorityId;
}
