use std::collections::HashMap;

use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthoritySignature};
use sp_runtime::traits::Block;

use crate::{
	message::{Timeout, Vote, QC, TC},
	primitives::{HotstuffError, HotstuffError::*, ViewNumber},
};

pub struct Aggregator<B: Block> {
	votes_aggregator: HashMap<ViewNumber, HashMap<B::Hash, QCMaker>>,
	timeouts_aggregators: HashMap<ViewNumber, TCMaker>,
}

impl<B: Block> Aggregator<B> {
	pub fn new() -> Self {
		Self { votes_aggregator: HashMap::new(), timeouts_aggregators: HashMap::new() }
	}

	pub fn add_vote(
		&mut self,
		vote: Vote<B>,
		authorities: &AuthorityList,
	) -> Result<Option<QC<B>>, HotstuffError> {
		self.votes_aggregator
			.entry(vote.view)
			.or_insert_with(HashMap::new)
			.entry(vote.digest())
			.or_insert_with(|| QCMaker::new())
			.append(vote, authorities)
	}

	pub fn add_timeout(
		&mut self,
		timeout: Timeout<B>,
		authorities: &AuthorityList,
	) -> Result<Option<TC<B>>, HotstuffError> {
		// Add the new timeout to our aggregator and see if we have a TC.
		self.timeouts_aggregators
			.entry(timeout.view)
			.or_insert_with(|| TCMaker::new())
			.append(timeout, authorities)
	}
}

pub struct QCMaker {
	weight: u64,
	votes: Vec<(AuthorityId, AuthoritySignature)>,
}

impl QCMaker {
	pub fn new() -> Self {
		QCMaker { weight: 0, votes: Vec::new() }
	}

	pub fn append<B: Block>(
		&mut self,
		vote: Vote<B>,
		authorities: &AuthorityList,
	) -> Result<Option<QC<B>>, HotstuffError> {
		let voter = vote.voter;

		// TODO check voter weather in authorities?

		self.votes
			.iter()
			.find(|(id, _)| id.eq(&voter))
			.ok_or_else(|| AuthorityReuse(voter.to_owned()))?;

		self.votes.push((voter, vote.signature));
		self.weight += 1;

		if self.weight < 4 || self.weight < (authorities.len() * 2 / 3 + 1) as u64 {
			return Ok(None)
		}

		Ok(Some(QC::<B> { hash: vote.hash, view: vote.view, votes: self.votes.clone() }))
	}
}

pub struct TCMaker {
	weight: u64,
	votes: Vec<(AuthorityId, AuthoritySignature, ViewNumber)>,
}

impl TCMaker {
	pub fn new() -> Self {
		Self { weight: 0, votes: Vec::new() }
	}

	pub fn append<B: Block>(
		&mut self,
		timeout: Timeout<B>,
		authorities: &AuthorityList,
	) -> Result<Option<TC<B>>, HotstuffError> {
		let voter = timeout.voter;
		// TODO check voter weather in authorities?

		self.votes
			.iter()
			.find(|(id, _, _)| id.eq(&voter))
			.ok_or_else(|| AuthorityReuse(voter.to_owned()))?;

		self.votes.push((voter, timeout.signature.ok_or(NullSignature)?, timeout.view));
		self.weight += 1;

		if self.weight < 4 || self.weight < (authorities.len() * 2 / 3 + 1) as u64 {
			return Ok(None)
		}

		Ok(Some(TC::<B> {
			view: timeout.view,
			votes: self.votes.clone(),
			_phantom: std::marker::PhantomData,
		}))
	}
}
