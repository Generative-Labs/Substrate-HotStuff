use std::{
	future::Future,
	pin::Pin,
	sync::Arc,
	task::{Context, Poll},
	time::Duration,
};

use log::{debug, info};
use tokio::time::{interval, Instant, Interval};

use sc_client_api::Backend;
use sp_core::{Decode, Encode};
use sp_runtime::traits::Block as BlockT;

use crate::{
	client::ClientForHotstuff, message::Proposal, primitives::HotstuffError, store::Store,
};

pub struct Timer {
	delay: Interval,
}

impl Timer {
	pub fn new(duration: u64) -> Self {
		Self { delay: interval(Duration::from_millis(duration)) }
	}

	pub fn reset(&mut self) {
		self.delay.reset();
	}
}

impl Future for Timer {
	type Output = Instant;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.delay.poll_tick(cx)
	}
}

// Synchronizer synchronizes replicas to the same view.
pub struct Synchronizer<B: BlockT, BE: Backend<B>, C: ClientForHotstuff<B, BE>> {
	store: Store<B, BE, C>,
}

impl<B, BE, C> Synchronizer<B, BE, C>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
{
	pub fn new(client: Arc<C>) -> Self {
		Self { store: Store::new(client) }
	}

	pub fn save_proposal(&mut self, proposal: &Proposal<B>) -> Result<(), HotstuffError> {
		let value = proposal.encode();
		let key = proposal.digest();

		info!("~~ save proposal, digest {}", key);

		self.store
			.set(key.as_ref(), &value)
			.map_err(|e| HotstuffError::SaveProposal(e.to_string()))
	}

	// pub fn get_proposal(&mut self, hash: B::Hash) {}

	pub fn get_proposal_ancestors(
		&self,
		proposal: &Proposal<B>,
	) -> Result<(Proposal<B>, Proposal<B>), HotstuffError> {
		debug!(
			"~~ get_proposal_ancestors, for proposal {:#?}, parent {:#?}",
			proposal.digest(),
			proposal.parent_hash()
		);

		let parent = self.get_proposal_parent(proposal)?;

		debug!(
			"~~ get_proposal_ancestors has parent, for proposal {:#?}, parent {:#?}",
			proposal.digest(),
			proposal.parent_hash()
		);

		let grandpa = self.get_proposal_parent(&parent)?;

		debug!(
			"~~ get_proposal_ancestors has grandpa, for proposal {:#?}, parent {:#?}",
			proposal.digest(),
			proposal.parent_hash()
		);

		info!("~~ get_proposal_ancestors, parent {:#?}, grandpa {:#?}", parent, grandpa);

		Ok((parent, grandpa))
	}

	pub fn get_proposal_parent(
		&self,
		proposal: &Proposal<B>,
	) -> Result<Proposal<B>, HotstuffError> {
		let res = self
			.store
			.get(proposal.parent_hash().as_ref())
			.map_err(|e| HotstuffError::Other(e.to_string()))?;

		if let Some(data) = res {
			let proposal: Proposal<B> =
				Decode::decode(&mut &data[..]).map_err(|e| HotstuffError::Other(e.to_string()))?;
			return Ok(proposal)
		}

		// TODO request from network, wait result here?

		Err(HotstuffError::ProposalNotFound)
	}
}
