use std::cmp::max;

use parity_scale_codec::Encode;

use log::{info, error};
use sc_client_api::Backend;
use sp_application_crypto::AppCrypto;
use sp_consensus_hotstuff::{AuthorityId, AuthorityList, AuthoritySignature, HOTSTUFF_KEY_TYPE};
use sp_core::crypto::ByteArray;
use sp_keystore::KeystorePtr;
use sp_runtime::traits::Block as BlockT;

use crate::{
	aggregator::Aggregator,
	client::ClientForHotstuff,
	message::{ConsensusMessage, Timeout, QC, TC},
	network_bridge::{HotstuffNetworkBridge, Network, Syncing},
	primitives::{HotstuffError, HotstuffError::*, ViewNumber},
	synchronizer::{Synchronizer, Timer},
};

// the core of hotstuff
pub struct Consensus<B: BlockT, BE: Backend<B>, C: ClientForHotstuff<B, BE>> {
	keystore: KeystorePtr,
	authorities: AuthorityList,
	synchronizer: Synchronizer<B, BE, C>,
	view: ViewNumber,
	last_voted_view: ViewNumber,
	last_committed_round: ViewNumber,
	high_qc: QC<B>,
	aggregator: Aggregator<B>,
}

impl<B, BE, C> Consensus<B, BE, C>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
{
	// find local authority id. If the result is None, local node is not authority.
	pub fn local_authority_id(&self) -> Option<AuthorityId> {
		self.authorities
			.iter()
			.find(|(p, _)| self.keystore.has_keys(&[(p.to_raw_vec(), HOTSTUFF_KEY_TYPE)]))
			.map(|(p, _)| p.clone())
	}

	pub fn increase_last_voted_view(&mut self) {
		self.last_voted_view = max(self.last_voted_view, self.view)
	}

	pub fn make_timeout(&self) -> Result<Timeout<B>, HotstuffError> {
		let authority_id = self.local_authority_id().ok_or(NotAuthority)?;

		let mut tc: Timeout<B> = Timeout {
			high_qc: self.high_qc.clone(),
			view: self.view,
			voter: authority_id.clone(),
			signature: None,
		};

		tc.signature = self
			.keystore
			.sign_with(
				AuthorityId::ID,
				AuthorityId::CRYPTO_ID,
				authority_id.as_slice(),
				tc.digest().as_ref(),
			)
			.map_err(|e| Other(e.to_string()))?
			.and_then(|data| AuthoritySignature::try_from(data).ok());

		Ok(tc)
	}

    pub fn handle_timeout(&mut self, timeout: &Timeout<B>)->Result<Option<TC<B>>, HotstuffError>{
        Ok(None)
    }
}

pub struct ConsensusWorker<
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: Network<B> + Sync + 'static,
	S: Syncing<B> + Sync + 'static,
> {
	consensus_state: Consensus<B, BE, C>,

	network: HotstuffNetworkBridge<B, N, S>,
	local_timer: Timer,
}

impl<B, BE, C, N, S> ConsensusWorker<B, BE, C, N, S>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
	N: Network<B> + Sync + 'static,
	S: Syncing<B> + Sync + 'static,
{
    pub fn new(
        consensus_state: Consensus<B, BE, C>,
        network: HotstuffNetworkBridge<B, N, S>,
        local_timer_duration: u64,
    )->Self{
        Self { consensus_state, network, local_timer: Timer::new(local_timer_duration) }
    }

	pub async fn run(&mut self) {
		loop {
			tokio::select! {
				_ = &mut self.local_timer =>{
					if let Err(e) = self.handle_local_timeout(){
                        error!("handle local timeout has error {:#?}", e)
                    }
				}
			};
		}
	}

	pub fn handle_local_timeout(&mut self) -> Result<(), HotstuffError> {
		info!("local timeout, id: {}", self.network.local_peer_id());

		self.consensus_state.increase_last_voted_view();
		
        let timeout = self.consensus_state.make_timeout()?;
		let message = ConsensusMessage::Timeout(timeout.clone());

		self.network
			.gossip_engine
			.lock()
			.register_gossip_message(ConsensusMessage::<B>::gossip_topic(), message.encode());

        self.handle_timeout(&timeout);

		Ok(())
	}

    pub fn handle_timeout(&mut self, timeout: &Timeout<B>)->Result<(), HotstuffError>{
        
        Ok(())
    }
}
