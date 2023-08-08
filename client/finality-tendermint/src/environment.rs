use std::{
	collections::{BTreeMap, HashMap},
	marker::PhantomData,
	pin::Pin,
	sync::Arc,
};

use crate::{
	authorities::{AuthoritySet, SharedAuthoritySet},
	communication::{Network as NetworkT, Syncing as SyncingT},
	justification::TendermintJustification,
	local_authority_id,
	notification::TendermintJustificationSender,
	until_imported::UntilVoteTargetImported,
	ClientForTendermint, CommandOrError, Config, Error, FinalizedCommit, NewAuthoritySet,
	Precommit, Prevote, Proposal, SignedMessage, VoterCommand,
};
use finality_tendermint::{
	environment, messages, voter, BlockNumberOps, Error as TendermintError, VoterSet,
};
use futures::{prelude::*, Future, Sink, Stream};
use log::{debug, warn};
use parity_scale_codec::{Decode, Encode};
use parking_lot::RwLock;
use prometheus_endpoint::{register, Counter, Gauge, PrometheusError, U64};
use sc_client_api::{apply_aux, backend::Backend as BackendT, utils::is_descendent_of};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_DEBUG, CONSENSUS_INFO};
use sp_consensus::SelectChain as SelectChainT;
use sp_finality_tendermint::{
	AuthorityId, AuthoritySignature, RoundNumber, SetId, TendermintApi, TDMT_ENGINE_ID,
};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Header as HeaderT, NumberFor, Zero},
};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;

type SignedCommit<Block> = messages::SignedCommit<
	NumberFor<Block>,
	<Block as BlockT>::Hash,
	AuthoritySignature,
	AuthorityId,
>;

type HistoricalVotes<Block> = Vec<SignedCommit<Block>>;

// /// The environment we run TDMT in.
// pub(crate) struct Environment<Backend, Block: BlockT, C, N: NetworkT<Block>, SC> {
// 	pub(crate) client: Arc<C>,
// 	pub(crate) select_chain: SC,
// 	pub(crate) voters: Arc<VoterSet<AuthorityId>>,
// 	pub(crate) config: Config,
// 	pub(crate) authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
// 	pub(crate) network: crate::communication::NetworkBridge<Block, N, S>,
// 	pub(crate) set_id: SetId,
// 	pub(crate) voter_set_state: SharedVoterSetState<Block>,
// 	pub(crate) metrics: Option<Metrics>,
// 	pub(crate) justification_sender: Option<TendermintJustificationSender<Block>>,
// 	pub(crate) telemetry: Option<TelemetryHandle>,
// 	pub(crate) _phantom: PhantomData<Backend>,
// }

pub(crate) struct Environment<
    Backend,
    Block: BlockT,
    C,
    N: NetworkT<Block>,
    S: SyncingT<Block>,
    SC,
    VR,
> {
    pub(crate) client: Arc<C>,
    pub(crate) select_chain: SC,
    pub(crate) voters: Arc<VoterSet<AuthorityId>>,
    pub(crate) config: Config,
    pub(crate) authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
    pub(crate) network: crate::communication::NetworkBridge<Block, N, S>,
    pub(crate) set_id: SetId,
    pub(crate) voter_set_state: SharedVoterSetState<Block>,
    pub(crate) voting_rule: VR,
    pub(crate) metrics: Option<Metrics>,
    pub(crate) justification_sender: Option<TendermintJustificationSender<Block>>,
    pub(crate) telemetry: Option<TelemetryHandle>,
    pub(crate) offchain_tx_pool_factory: OffchainTransactionPoolFactory<Block>,
    pub(crate) _phantom: PhantomData<Backend>,
}

impl<BE, Block: BlockT, C, N: NetworkT<Block>, S: SyncingT<Block>, SC, VR> Environment<BE, Block, C, N, S, SC, VR> {
	/// Updates the voter set state using the given closure. The write lock is
	/// held during evaluation of the closure and the environment's voter set
	/// state is set to its result if successful.
	pub(crate) fn update_voter_set_state<F>(&self, f: F) -> Result<(), Error>
	where
		F: FnOnce(&VoterSetState<Block>) -> Result<Option<VoterSetState<Block>>, Error>,
	{
		self.voter_set_state.with(|voter_set_state| {
			if let Some(set_state) = f(&voter_set_state)? {
				*voter_set_state = set_state;

				if let Some(metrics) = self.metrics.as_ref() {
					if let VoterSetState::Live { completed_rounds, .. } = voter_set_state {
						let highest = completed_rounds
							.rounds
							.iter()
							.map(|round| round.number)
							.max()
							.expect("There is always one completed round (genesis); qed");

						metrics.finality_tendermint_round.set(highest);
					}
				}
			}
			Ok(())
		})
	}
}

impl<BE, Block, C, N, S, SC, VR>  environment::Environment for Environment<BE, Block, C, N, S, SC, VR>
where
	Block: BlockT,
	BE: BackendT<Block>,
	C: ClientForTendermint<Block, BE> + 'static,
	C::Api: TendermintApi<Block>,
	N: NetworkT<Block>,
	S: SyncingT<Block>,
	SC: SelectChainT<Block> + 'static,
	NumberFor<Block>: BlockNumberOps,
{
	type Timer = Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send>>;
	type BestChain = Pin<
		Box<
			dyn Future<Output = Result<Option<(NumberFor<Block>, Block::Hash)>, Self::Error>>
				+ Send,
		>,
	>;

	type Id = AuthorityId;

	type Signature = AuthoritySignature;

	type In = Pin<
		Box<
			dyn Stream<
					Item = Result<
						messages::SignedMessage<
							NumberFor<Block>,
							Block::Hash,
							Self::Signature,
							Self::Id,
						>,
						Self::Error,
					>,
				> + Send,
		>,
	>;

	type Out = Pin<
		Box<
			dyn Sink<
					finality_tendermint::messages::Message<NumberFor<Block>, Block::Hash>,
					Error = Self::Error,
				> + Send,
		>,
	>;

	type Error = CommandOrError<Block::Hash, NumberFor<Block>>;

	type Hash = Block::Hash;

	type Number = NumberFor<Block>;

	type GlobalIn = Box<
		dyn Stream<
				Item = Result<
					messages::GlobalMessageIn<Self::Hash, Self::Number, Self::Signature, Self::Id>,
					Self::Error,
				>,
			> + Unpin
			+ Send,
	>;
	type GlobalOut = Pin<
		Box<
			dyn Sink<
					messages::GlobalMessageOut<Self::Hash, Self::Number, Self::Signature, Self::Id>,
					Error = Self::Error,
				> + Send,
		>,
	>;

	fn init_voter(&self) -> environment::VoterData<Self::Id> {
		let local_id = local_authority_id(&self.voters, self.config.keystore.as_ref())
			.expect("expect to have local_id to be a validtor.");

		environment::VoterData { local_id }
	}

	fn init_round(&self, round: u64) -> environment::RoundData<Self::Id, Self::In, Self::Out> {
		let local_id = local_authority_id(&self.voters, self.config.keystore.as_ref());

		let has_voted = match self.voter_set_state.has_voted(round) {
			HasVoted::Yes(id, vote) =>
				if local_id.as_ref().map(|k| k == &id).unwrap_or(false) {
					HasVoted::Yes(id, vote)
				} else {
					HasVoted::No
				},
			HasVoted::No => HasVoted::No,
		};

		// NOTE: we cache the local authority id that we'll be using to vote on the
		// given round. this is done to make sure we only check for available keys
		// from the keystore in this method when beginning the round, otherwise if
		// the keystore state changed during the round (e.g. a key was removed) it
		// could lead to internal state inconsistencies in the voter environment
		// (e.g. we wouldn't update the voter set state after prevoting since there's
		// no local authority id).
		if let Some(id) = local_id.as_ref() {
			self.voter_set_state.started_voting_on(round, id.clone());
		}
		// we can only sign when we have a local key in the authority set
		// and we have a reference to the keystore.
		let keystore = match (local_id.as_ref(), self.config.keystore.as_ref()) {
			(Some(id), Some(keystore)) => Some((id.clone(), keystore.clone()).into()),
			_ => None,
		};

		let (incoming, outgoing) = self.network.round_communication(
			keystore,
			crate::communication::Round(round),
			crate::communication::SetId(self.set_id),
			self.voters.clone(),
			has_voted,
		);

		// schedule incoming messages from the network to be held until
		// corresponding blocks are imported.
		let incoming = Box::pin(
			UntilVoteTargetImported::new(
				self.client.import_notification_stream(),
				self.network.clone(),
				self.client.clone(),
				incoming,
				"round",
				None,
			)
			.map_err(Into::into),
		);

		// schedule network message cleanup when sink drops.
		let outgoing = Box::pin(outgoing.sink_err_into());
		environment::RoundData { local_id: local_id.unwrap(), incoming, outgoing }
	}

	fn propose(&self, round: u64, block: Self::Hash) -> Self::BestChain {
		let client = self.client.clone();
		let authority_set = self.authority_set.clone();
		let select_chain = self.select_chain.clone();
		let set_id = self.set_id;
		Box::pin(async move {
			// NOTE: when we finalize an authority set change through the sync protocol the voter is
			//       signaled asynchronously. therefore the voter could still vote in the next round
			//       before activating the new set. the `authority_set` is updated immediately thus
			//       we restrict the voter based on that.
			if set_id != authority_set.set_id() {
				return Ok(None)
			}

			// FIXME: error
			next_target(block, client, authority_set, select_chain)
				.await
				.map_err(|e| e.into())
		})
	}

	// fn complete_f_commit(
	// 	&self,
	// 	round: u64,
	// 	state: (Self::Number, Self::Hash),
	// 	base: (Self::Number, Self::Hash),
	// 	f_commit: messages::FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
	// ) -> Result<(), Self::Error> {
	// 	self.update_voter_set_state(|voter_set_state| {
	// 		let (completed_rounds, current_rounds) =
	// 			if let VoterSetState::Live { completed_rounds, current_rounds } = voter_set_state {
	// 				(completed_rounds, current_rounds)
	// 			} else {
	// 				let msg = "Voter acting while in paused state.";
	// 				return Err(Error::Safety(msg.to_string()))
	// 			};
	//
	// 		let mut completed_rounds = completed_rounds.clone();
	// 		let mut current_rounds = current_rounds.clone();
	//
	// 		let votes = f_commit.commits;
	//
	// 		if let Some(_current_round) = current_rounds.get(&round) {
	// 			// Currently, we don't have to use `CurrentRounds`
	// 		} else {
	// 			current_rounds.insert(round, HasVoted::No);
	// 			if round >= 1 {
	// 				// Remove prerounds round if higher round present.
	// 				current_rounds.remove(&(round - 1));
	// 			}
	// 		}
	//
	// 		completed_rounds.push(CompletedRound {
	// 			number: round,
	// 			state: state.clone(),
	// 			base,
	// 			votes,
	// 		});
	//
	// 		let set_state = VoterSetState::<Block>::Live {
	// 			completed_rounds,
	// 			current_rounds: current_rounds.clone(),
	// 		};
	//
	// 		crate::aux_schema::write_voter_set_state(&*self.client, &set_state)?;
	//
	// 		Ok(Some(set_state))
	// 	})?;
	//
	// 	Ok(())
	// }

	fn finalize_block(
		&self,
		round: u64,
		hash: Self::Hash,
		number: Self::Number,
		f_commit: FinalizedCommit<Block>,
	) -> Result<(), Self::Error> {
		finalize_block(
			self.client.clone(),
			&self.authority_set,
			Some(self.config.justification_period.into()),
			hash,
			number,
			(round, f_commit).into(),
			false,
			self.justification_sender.as_ref(),
			self.telemetry.clone(),
		)
	}
}

async fn next_target<Block, Backend, Client, SelectChain>(
	block: Block::Hash,
	client: Arc<Client>,
	authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
	select_chain: SelectChain,
) -> Result<Option<(NumberFor<Block>, Block::Hash)>, Error>
where
	Backend: BackendT<Block>,
	Block: BlockT,
	Client: ClientForTendermint<Block, Backend>,
	SelectChain: SelectChainT<Block> + 'static,
{
	let base_header = match client.header(BlockId::Hash(block))? {
		Some(h) => h,
		None => {
			debug!(target: "afp",
				"Encountered error finding best chain containing {:?}: couldn't find base block",
				block,
			);

			return Ok(None)
		},
	};

	let result = match select_chain.finality_target(block, None).await {
		Ok(best_hash) => {
			let best_header = client
				.header(BlockId::Hash(best_hash))?
				.expect("Header known to exist after `finality_target` call; qed");

			if best_header == base_header {
				Some(best_header)
			} else {
				let mut target_header = best_header.clone();
				let mut parent_header = client
					.header(BlockId::Hash(*target_header.parent_hash()))?
					.expect("Header known to exist after `finality_target` call; qed");

				// walk backwards until we find the target block
				loop {
					if parent_header.number() < base_header.number() {
						unreachable!(
							"we are traversing backwards from a known block; \
                         blocks are stored contiguously; \
                         qed"
						);
					}

					if base_header.number() == parent_header.number() {
						break
					}

					target_header = client
						.header(BlockId::Hash(*target_header.parent_hash()))?
						.expect("Header known to exist after `finality_target` call; qed");
					parent_header = client
						.header(BlockId::Hash(*target_header.parent_hash()))?
						.expect("Header known to exist after `finality_target` call; qed");
				}

				Some(target_header)
			}
		},
		Err(e) => {
			warn!(target: "afp", "Encountered error finding best chain containing {:?}: {}", block, e);
			None
		},
	};

	Ok(result.map(|h| (*h.number(), h.hash())))
}

/// Whether we've voted already during a prior run of the program.
#[derive(Clone, Debug, Decode, Encode, PartialEq)]
pub enum HasVoted<Block: BlockT> {
	/// Has not voted already in this round.
	No,
	/// Has voted in this round.
	Yes(AuthorityId, Vote<Block>),
}

/// The votes cast by this voter already during a prior run of the program.
#[derive(Debug, Clone, Decode, Encode, PartialEq)]
pub enum Vote<Block: BlockT> {
	/// Has cast a proposal.
	Proposal(Proposal<Block>),
	/// Has cast a prevote.
	Prevote(Option<Proposal<Block>>, Prevote<Block>),
	/// Has cast a precommit (implies prevote.)
	Precommit(Option<Proposal<Block>>, Prevote<Block>, Precommit<Block>),
}

impl<Block: BlockT> HasVoted<Block> {
	/// Returns the proposal we should vote with (if any.)
	pub fn proposal(&self) -> Option<&Proposal<Block>> {
		match self {
			HasVoted::Yes(_, Vote::Proposal(propose)) => Some(propose),
			HasVoted::Yes(_, Vote::Prevote(propose, _)) |
			HasVoted::Yes(_, Vote::Precommit(propose, _, _)) => propose.as_ref(),
			_ => None,
		}
	}

	/// Returns the prevote we should vote with (if any.)
	pub fn prevote(&self) -> Option<&Prevote<Block>> {
		match self {
			HasVoted::Yes(_, Vote::Prevote(_, prevote)) |
			HasVoted::Yes(_, Vote::Precommit(_, prevote, _)) => Some(prevote),
			_ => None,
		}
	}

	/// Returns the precommit we should vote with (if any.)
	pub fn precommit(&self) -> Option<&Precommit<Block>> {
		match self {
			HasVoted::Yes(_, Vote::Precommit(_, _, commit)) => Some(commit),
			_ => None,
		}
	}

	/// FIXME: Returns true if the voter can still propose, false otherwise.
	pub fn can_proposal(&self) -> bool {
		self.proposal().is_none()
	}

	/// Returns true if the voter can still prevote, false otherwise.
	pub fn can_prevote(&self) -> bool {
		self.prevote().is_none()
	}

	/// Returns true if the voter can still precommit, false otherwise.
	pub fn can_commit(&self) -> bool {
		self.precommit().is_none()
	}
}

/// A map with voter status information for currently live rounds,
/// which votes have we cast and what are they.
pub type CurrentRounds<Block> = BTreeMap<RoundNumber, HasVoted<Block>>;

/// Data about a completed round. The set of votes that is stored must be
/// minimal, i.e. at most one equivocation is stored per voter.
#[derive(Debug, Clone, Decode, Encode, PartialEq)]
pub struct CompletedRound<Block: BlockT> {
	/// The round number.
	pub number: RoundNumber,
	/// The round state (prevote ghost, estimate, finalized, etc.)
	pub state: (NumberFor<Block>, Block::Hash),
	/// The target block base used for voting in the round.
	pub base: (NumberFor<Block>, Block::Hash),
	/// All the votes observed in the round.
	pub votes: Vec<SignedCommit<Block>>,
}

// Data about last completed rounds within a single voter set. Stores
// NUM_LAST_COMPLETED_ROUNDS and always contains data about at least one round
// (genesis).
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedRounds<Block: BlockT> {
	rounds: Vec<CompletedRound<Block>>,
	set_id: SetId,
	voters: Vec<AuthorityId>,
}

const NUM_LAST_COMPLETED_VIEWS: usize = 2;

impl<Block: BlockT> Encode for CompletedRounds<Block> {
	fn encode(&self) -> Vec<u8> {
		let v = Vec::from_iter(&self.rounds);
		(&v, &self.set_id, &self.voters).encode()
	}
}

impl<Block: BlockT> parity_scale_codec::EncodeLike for CompletedRounds<Block> {}

impl<Block: BlockT> Decode for CompletedRounds<Block> {
	fn decode<I: parity_scale_codec::Input>(
		value: &mut I,
	) -> Result<Self, parity_scale_codec::Error> {
		<(Vec<CompletedRound<Block>>, SetId, Vec<AuthorityId>)>::decode(value)
			.map(|(rounds, set_id, voters)| CompletedRounds { rounds, set_id, voters })
	}
}

impl<Block: BlockT> CompletedRounds<Block> {
	/// Create a new completed rounds tracker with NUM_LAST_COMPLETED_ROUNDS capacity.
	pub(crate) fn new(
		genesis: CompletedRound<Block>,
		set_id: SetId,
		voters: &AuthoritySet<Block::Hash, NumberFor<Block>>,
	) -> CompletedRounds<Block> {
		let mut rounds = Vec::with_capacity(NUM_LAST_COMPLETED_VIEWS);
		rounds.push(genesis);

		let voters = voters.current_authorities.iter().map(|a| a.clone()).collect();
		CompletedRounds { rounds, set_id, voters }
	}

	/// Get the set-id and voter set of the completed rounds.
	pub fn set_info(&self) -> (SetId, &[AuthorityId]) {
		(self.set_id, &self.voters[..])
	}

	/// Iterate over all completed rounds.
	pub fn iter(&self) -> impl Iterator<Item = &CompletedRound<Block>> {
		self.rounds.iter().rev()
	}

	/// Returns the last (latest) completed round.
	pub fn last(&self) -> &CompletedRound<Block> {
		self.rounds
			.first()
			.expect("inner is never empty; always contains at least genesis; qed")
	}

	/// Push a new completed round, oldest round is evicted if number of rounds
	/// is higher than `NUM_LAST_COMPLETED_ROUNDS`.
	pub fn push(&mut self, completed_round: CompletedRound<Block>) {
		use std::cmp::Reverse;

		match self
			.rounds
			.binary_search_by_key(&Reverse(completed_round.number), |completed_round| {
				Reverse(completed_round.number)
			}) {
			Ok(idx) => self.rounds[idx] = completed_round,
			Err(idx) => self.rounds.insert(idx, completed_round),
		};

		if self.rounds.len() > NUM_LAST_COMPLETED_VIEWS {
			self.rounds.pop();
		}
	}
}

/// The state of the current voter set, whether it is currently active or not
/// and information related to the previously completed rounds. Current round
/// voting status is used when restarting the voter, i.e. it will re-use the
/// previous votes for a given round if appropriate (same round and same local
/// key).
#[derive(Debug, Decode, Encode, PartialEq)]
pub enum VoterSetState<Block: BlockT> {
	/// The voter is live, i.e. participating in rounds.
	Live {
		/// The previously completed rounds.
		completed_rounds: CompletedRounds<Block>,
		/// Voter status for the currently live rounds.
		current_rounds: CurrentRounds<Block>,
	},
	/// The voter is paused, i.e. not casting or importing any votes.
	Paused {
		/// The previously completed rounds.
		completed_rounds: CompletedRounds<Block>,
	},
}

impl<Block: BlockT> VoterSetState<Block> {
	/// Create a new live VoterSetState with round 0 as a completed round using
	/// the given genesis state and the given authorities. Round 1 is added as a
	/// current round (with state `HasVoted::No`).
	pub(crate) fn live(
		set_id: SetId,
		authority_set: &AuthoritySet<Block::Hash, NumberFor<Block>>,
		genesis_state: (Block::Hash, NumberFor<Block>),
	) -> VoterSetState<Block> {
		let state = (genesis_state.1, genesis_state.0);
		let completed_rounds = CompletedRounds::new(
			CompletedRound {
				number: 0,
				state,
				base: (genesis_state.1, genesis_state.0),
				votes: Vec::new(),
			},
			set_id,
			authority_set,
		);

		let mut current_rounds = CurrentRounds::new();
		current_rounds.insert(1, HasVoted::No);

		VoterSetState::Live { completed_rounds, current_rounds }
	}

	/// Returns the last completed rounds.
	pub(crate) fn completed_rounds(&self) -> CompletedRounds<Block> {
		match self {
			VoterSetState::Live { completed_rounds, .. } => completed_rounds.clone(),
			VoterSetState::Paused { completed_rounds } => completed_rounds.clone(),
		}
	}

	/// Returns the last completed round.
	pub(crate) fn last_completed_round(&self) -> CompletedRound<Block> {
		match self {
			VoterSetState::Live { completed_rounds, .. } => completed_rounds.last().clone(),
			VoterSetState::Paused { completed_rounds } => completed_rounds.last().clone(),
		}
	}

	/// Returns the voter set state validating that it includes the given round
	/// in current rounds and that the voter isn't paused.
	pub fn with_current_round(
		&self,
		round: RoundNumber,
	) -> Result<(&CompletedRounds<Block>, &CurrentRounds<Block>), Error> {
		if let VoterSetState::Live { completed_rounds, current_rounds } = self {
			if current_rounds.contains_key(&round) {
				Ok((completed_rounds, current_rounds))
			} else {
				let msg = "Voter acting on a live round we are not tracking.";
				Err(Error::Safety(msg.to_string()))
			}
		} else {
			let msg = "Voter acting while in paused state.";
			Err(Error::Safety(msg.to_string()))
		}
	}
}
/// A voter set state meant to be shared safely across multiple owners.
#[derive(Clone)]
pub struct SharedVoterSetState<Block: BlockT> {
	/// The inner shared `VoterSetState`.
	inner: Arc<RwLock<VoterSetState<Block>>>,
	/// A tracker for the rounds that we are actively participating on (i.e. voting)
	/// and the authority id under which we are doing it.
	voting: Arc<RwLock<HashMap<RoundNumber, AuthorityId>>>,
}

impl<Block: BlockT> From<VoterSetState<Block>> for SharedVoterSetState<Block> {
	fn from(set_state: VoterSetState<Block>) -> Self {
		SharedVoterSetState::new(set_state)
	}
}

impl<Block: BlockT> SharedVoterSetState<Block> {
	/// Create a new shared voter set tracker with the given state.
	pub(crate) fn new(set_state: VoterSetState<Block>) -> Self {
		SharedVoterSetState {
			inner: Arc::new(RwLock::new(set_state)),
			voting: Arc::new(RwLock::new(HashMap::new())),
		}
	}

	/// Read the inner voter set state.
	pub(crate) fn read(&self) -> parking_lot::RwLockReadGuard<VoterSetState<Block>> {
		self.inner.read()
	}

	/// Get the authority id that we are using to vote on the given round, if any.
	pub(crate) fn voting_on(&self, round: RoundNumber) -> Option<AuthorityId> {
		self.voting.read().get(&round).cloned()
	}

	/// Note that we started voting on the give round with the given authority id.
	pub(crate) fn started_voting_on(&self, round: RoundNumber, local_id: AuthorityId) {
		self.voting.write().insert(round, local_id);
	}

	/// Note that we have finished voting on the given round. If we were voting on
	/// the given round, the authority id that we were using to do it will be
	/// cleared.
	pub(crate) fn finished_voting_on(&self, round: RoundNumber) {
		self.voting.write().remove(&round);
	}

	/// Return vote status information for the current round.
	pub(crate) fn has_voted(&self, round: RoundNumber) -> HasVoted<Block> {
		match &*self.inner.read() {
			VoterSetState::Live { current_rounds, .. } => current_rounds
				.get(&round)
				.and_then(|has_voted| match has_voted {
					HasVoted::Yes(id, vote) => Some(HasVoted::Yes(id.clone(), vote.clone())),
					_ => None,
				})
				.unwrap_or(HasVoted::No),
			_ => HasVoted::No,
		}
	}

	// NOTE: not exposed outside of this module intentionally.
	fn with<F, R>(&self, f: F) -> R
	where
		F: FnOnce(&mut VoterSetState<Block>) -> R,
	{
		f(&mut *self.inner.write())
	}
}

/// Prometheus metrics for TDMT.
#[derive(Clone)]
pub(crate) struct Metrics {
	finality_tendermint_round: Gauge<U64>,
	finality_tendermint_prevotes: Counter<U64>,
	finality_tendermint_commits: Counter<U64>,
}

impl Metrics {
	pub(crate) fn register(
		registry: &prometheus_endpoint::Registry,
	) -> Result<Self, PrometheusError> {
		Ok(Self {
			finality_tendermint_round: register(
				Gauge::new("substrate_finality_tendermint_round", "Highest completed TDMT round.")?,
				registry,
			)?,
			finality_tendermint_prevotes: register(
				Counter::new(
					"substrate_finality_tendermint_prevotes_total",
					"Total number of TDMT prevotes cast locally.",
				)?,
				registry,
			)?,
			finality_tendermint_commits: register(
				Counter::new(
					"substrate_finality_tendermint_commits_total",
					"Total number of GRANDPA commits cast locally.",
				)?,
				registry,
			)?,
		})
	}
}

pub(crate) enum JustificationOrCommit<Block: BlockT> {
	Justification(TendermintJustification<Block>),
	Commit((RoundNumber, FinalizedCommit<Block>)),
}

impl<Block: BlockT> From<(RoundNumber, FinalizedCommit<Block>)> for JustificationOrCommit<Block> {
	fn from(commit: (RoundNumber, FinalizedCommit<Block>)) -> JustificationOrCommit<Block> {
		JustificationOrCommit::Commit(commit)
	}
}

impl<Block: BlockT> From<TendermintJustification<Block>> for JustificationOrCommit<Block> {
	fn from(justification: TendermintJustification<Block>) -> JustificationOrCommit<Block> {
		JustificationOrCommit::Justification(justification)
	}
}

/// Finalize the given block and apply any authority set changes. If an
/// authority set change is enacted then a justification is created (if not
/// given) and stored with the block when finalizing it.
/// This method assumes that the block being finalized has already been imported.
pub(crate) fn finalize_block<BE, Block, Client>(
	client: Arc<Client>,
	authority_set: &SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
	justification_period: Option<NumberFor<Block>>,
	hash: Block::Hash,
	number: NumberFor<Block>,
	justification_or_commit: JustificationOrCommit<Block>,
	initial_sync: bool,
	justification_sender: Option<&TendermintJustificationSender<Block>>,
	telemetry: Option<TelemetryHandle>,
) -> Result<(), CommandOrError<Block::Hash, NumberFor<Block>>>
where
	Block: BlockT,
	BE: BackendT<Block>,
	Client: ClientForTendermint<Block, BE>,
{
	// NOTE: lock must be held through writing to DB to avoid race. this lock
	//       also implicitly synchronizes the check for last finalized number
	//       below.
	let mut authority_set = authority_set.inner();

	let status = client.info();

	if number <= status.finalized_number && client.hash()? == Some(hash) {
		// This can happen after a forced change (triggered manually from the runtime when
		// finality is stalled), since the voter will be restarted at the median last finalized
		// block, which can be lower than the local best finalized block.
		warn!(target: "afp", "Re-finalized block #{:?} ({:?}) in the canonical chain, current best finalized is #{:?}",
				hash,
				number,
				status.finalized_number,
		);

		return Ok(())
	}

	// FIXME #1483: clone only when changed
	let old_authority_set = authority_set.clone();

	let update_res: Result<_, Error> = client.lock_import_and_run(|import_op| {
		let status = authority_set
			.apply_standard_changes(
				hash,
				number,
				&is_descendent_of::<Block, _>(&*client, None),
				initial_sync,
				None,
			)
			.map_err(|e| Error::Safety(e.to_string()))?;

		// send a justification notification if a sender exists and in case of error log it.
		fn notify_justification<Block: BlockT>(
			justification_sender: Option<&TendermintJustificationSender<Block>>,
			justification: impl FnOnce() -> Result<TendermintJustification<Block>, Error>,
		) {
			if let Some(sender) = justification_sender {
				if let Err(err) = sender.notify(justification) {
					warn!(target: "afp", "Error creating justification for subscriber: {}", err);
				}
			}
		}

		// NOTE: this code assumes that honest voters will never vote past a
		// transition block, thus we don't have to worry about the case where
		// we have a transition with `effective_block = N`, but we finalize
		// `N+1`. this assumption is required to make sure we store
		// justifications for transition blocks which will be requested by
		// syncing clients.
		let (justification_required, justification) = match justification_or_commit {
			JustificationOrCommit::Justification(justification) => (true, justification),
			JustificationOrCommit::Commit((round_number, commit)) => {
				let mut justification_required =
					// justification is always required when block that enacts new authorities
					// set is finalized
					status.new_set_block.is_some();

				// justification is required every N blocks to be able to prove blocks
				// finalization to remote nodes
				if !justification_required {
					if let Some(justification_period) = justification_period {
						let last_finalized_number = client.info().finalized_number;
						justification_required = (!last_finalized_number.is_zero() ||
							number - last_finalized_number == justification_period) &&
							(last_finalized_number / justification_period !=
								number / justification_period);
					}
				}

				let justification =
					TendermintJustification::from_commit(&client, round_number, commit)?;

				(justification_required, justification)
			},
		};

		notify_justification(justification_sender, || Ok(justification.clone()));

		let persisted_justification = if justification_required {
			Some((TDMT_ENGINE_ID, justification.encode()))
		} else {
			None
		};

		// ideally some handle to a synchronization oracle would be used
		// to avoid unconditionally notifying.
		client
			.apply_finality(import_op, BlockId::Hash(hash), persisted_justification, true)
			.map_err(|e| {
				warn!(target: "afp", "Error applying finality to block {:?}: {}", (hash, number), e);
				e
			})?;

		debug!(target: "afp", "Finalizing blocks up to ({:?}, {})", number, hash);

		telemetry!(
			telemetry;
			CONSENSUS_INFO;
			"afp.finalized_blocks_up_to";
			"number" => ?number, "hash" => ?hash,
		);

		crate::aux_schema::update_best_justification(&justification, |insert| {
			apply_aux(import_op, insert, &[])
		})?;

		let new_authorities = if let Some((canon_hash, canon_number)) = status.new_set_block {
			// the authority set has changed.
			let (new_id, set_ref) = authority_set.current();

			if set_ref.len() > 16 {
				afp_log!(
					initial_sync,
					"👴 Applying GRANDPA set change to new set with {} authorities",
					set_ref.len(),
				);
			} else {
				afp_log!(initial_sync, "👴 Applying GRANDPA set change to new set {:?}", set_ref);
			}

			telemetry!(
				telemetry;
				CONSENSUS_INFO;
				"afp.generating_new_authority_set";
				"number" => ?canon_number, "hash" => ?canon_hash,
				"authorities" => ?set_ref.to_vec(),
				"set_id" => ?new_id,
			);
			Some(NewAuthoritySet {
				canon_hash,
				canon_number,
				set_id: new_id,
				authorities: set_ref.to_vec(),
			})
		} else {
			None
		};

		if status.changed {
			let write_result = crate::aux_schema::update_authority_set::<Block, _, _>(
				&authority_set,
				new_authorities.as_ref(),
				|insert| apply_aux(import_op, insert, &[]),
			);

			if let Err(e) = write_result {
				warn!(target: "afp", "Failed to write updated authority set to disk. Bailing.");
				warn!(target: "afp", "Node is in a potentially inconsistent state.");

				return Err(e.into())
			}
		}

		Ok(new_authorities.map(VoterCommand::ChangeAuthorities))
	});

	match update_res {
		Ok(Some(command)) => Err(CommandOrError::VoterCommand(command)),
		Ok(None) => Ok(()),
		Err(e) => {
			*authority_set = old_authority_set;

			Err(CommandOrError::Error(e))
		},
	}
}
