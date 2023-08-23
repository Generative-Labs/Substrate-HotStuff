use futures::prelude::*;

use sp_api::ProvideRuntimeApi;
use sc_client_api::{backend::AuxStore, BlockOf};

use sp_runtime::traits::{Block as BlockT, Member, NumberFor};
use codec::Codec;
use sp_blockchain::HeaderBackend;
pub use sp_consensus::SyncOracle;

use sp_consensus::{ Environment, Error as ConsensusError, Proposer, SelectChain};
use sc_consensus::BlockImport;


use tokio::time::{sleep, Duration};
use sp_core::crypto::Pair;
use sp_application_crypto::AppPublic;


type AuthorityId<P> = <P as Pair>::Public;

pub mod params;

use sp_hotstuff::HotstuffApi;
use params::StartHotstuffParams;

struct HotstuffWork {
}

impl HotstuffWork {
    async fn do_work(&self) {
        // Implement your actual work logic here
        println!("Performing HotstuffWork...");
    }
}


pub fn start_hotstuff<P, B, C, SC, I, PF, SO, L, Error>(
	StartHotstuffParams {
		client: _,
		select_chain: _,
		block_import: _,
		proposer_factory: _,
		sync_oracle: _,
		justification_sync_link: _,
		force_authoring: _,
		keystore: _,
		telemetry: _,
		compatibility_mode: _,
	}: StartHotstuffParams<C, SC, I, PF, SO, L, NumberFor<B>>,
) -> Result<impl Future<Output = ()>, ConsensusError> 
where
	P: Pair,
	P::Public: AppPublic + Member,
	P::Signature: TryFrom<Vec<u8>> + Member + Codec,
	B: BlockT,
	C: ProvideRuntimeApi<B> + BlockOf + AuxStore + HeaderBackend<B> + Send + Sync,
	C::Api: HotstuffApi<B, AuthorityId<P>>,
	SC: SelectChain<B>,
	I: BlockImport<B, Transaction = sp_api::TransactionFor<C, B>> + Send + Sync + 'static,
	PF: Environment<B, Error = Error> + Send + Sync + 'static,
	PF::Proposer: Proposer<B, Error = Error, Transaction = sp_api::TransactionFor<C, B>>,
	SO: SyncOracle + Send + Sync + Clone,
	L: sc_consensus::JustificationSyncLink<B>,
	Error: std::error::Error + Send + From<ConsensusError> + 'static,
{
    println!("Hello hotstuff");

    let work = HotstuffWork {};

    let future_work = async move {
        loop {
            work.do_work().await;
            sleep(Duration::from_millis(3000)).await;
            println!("3000 ms have elapsed");
        }
    };

    Ok(future_work)
}

mod voter;


#[cfg(test)]
mod tests {

	use crate::voter::{HotstuffVoter, HotstuffLeader, VoteInfo};

    #[test]
    fn test_start_hotstuff() {
        // Arrange
        // let result = start_hotstuff().expect("hotstuff entry function");

        // Assert
        // Add your assertions here if needed

    }

	#[test]
	fn test_hotstuff_voter() {
		let voter = HotstuffVoter{};

		let vote = VoteInfo{};
		voter.vote_handler(&vote);
	}

}