
use sp_core::H256;
pub type SignatureBytes = [u8; 64];

pub type ViewNumber = u64;
pub type SignatureSet = Vec<Option<SignatureBytes>>;

pub enum Phase {
    Generic,
    Prepare,
    Precommit(ViewNumber),
    Commit(ViewNumber),
}

pub struct QuorumCertificate {
    pub view_number: ViewNumber,
    pub block_hash: H256,
    pub phase: Phase,
    pub signatures: SignatureSet,
}
