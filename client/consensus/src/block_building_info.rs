use crate::BlockT;
use codec::Encode;
use sc_cli::Result;
use sp_consensus_hotstuff::{Slot, SlotDuration, HOTSTUFF_ENGINE_ID};

use sp_inherents::{InherentData, InherentDataProvider};
use sp_runtime::{Digest, DigestItem};
use sp_timestamp::TimestampInherentData;

/// Something that can create inherent data providers and pre-runtime digest.
///
/// It is possible for the caller to provide custom arguments to the callee by setting the
/// `ExtraArgs` generic parameter.
///
/// This module already provides some convenience implementation of this trait for closures. So, it
/// should not be required to implement it directly.
#[async_trait::async_trait]
pub trait BlockBuildingInfoProvider<Block: BlockT, ExtraArgs = ()> {
	type InherentDataProviders: InherentDataProvider;

	// hello
	async fn get_inherent_providers_and_pre_digest(
		&self,
		parent_hash: Block::Hash,
		extra_args: ExtraArgs,
	) -> Result<(Self::InherentDataProviders, Vec<DigestItem>)>;
}

#[async_trait::async_trait]
impl<F, Block, IDP, ExtraArgs, Fut> BlockBuildingInfoProvider<Block, ExtraArgs> for F
where
	Block: BlockT,
	F: Fn(Block::Hash, ExtraArgs) -> Fut + Sync + Send,
	Fut: std::future::Future<Output = Result<(IDP, Vec<DigestItem>)>> + Send + 'static,
	IDP: InherentDataProvider + 'static,
	ExtraArgs: Send + 'static,
{
	type InherentDataProviders = IDP;

	async fn get_inherent_providers_and_pre_digest(
		&self,
		parent: Block::Hash,
		extra_args: ExtraArgs,
	) -> Result<(Self::InherentDataProviders, Vec<DigestItem>)> {
		(*self)(parent, extra_args).await
	}
}

/// Provides [`BlockBuildingInfoProvider`] implementation for chains that include timestamp inherent
/// and use Hotstuff for a block production.
///
/// It depends only on the expected block production frequency, i.e. `blocktime_millis`.
pub fn timestamp_with_hotstuff_info<Block: BlockT>(
	blocktime_millis: u64,
) -> impl BlockBuildingInfoProvider<Block, Option<(InherentData, Digest)>> {
	move |_, maybe_prev_info: Option<(InherentData, Digest)>| async move {
		let timestamp_idp = match maybe_prev_info {
			Some((inherent_data, _)) => sp_timestamp::InherentDataProvider::new(
				inherent_data.timestamp_inherent_data().unwrap().unwrap() + blocktime_millis,
			),
			None => sp_timestamp::InherentDataProvider::from_system_time(),
		};

		let slot =
			Slot::from_timestamp(*timestamp_idp, SlotDuration::from_millis(blocktime_millis));
		let digest = vec![DigestItem::PreRuntime(HOTSTUFF_ENGINE_ID, slot.encode())];

		Ok((timestamp_idp, digest))
	}
}
