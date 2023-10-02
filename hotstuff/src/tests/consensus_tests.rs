// // hotstuff worker tests

// use super::*;

// use sp_api::{ApiRef, ProvideRuntimeApi};
// use sc_network_test::{Block};

// use crate::client::GenesisAuthoritySetProvider;


// #[derive(Default, Clone)]
// pub(crate) struct TestApi {
// 	genesis_authorities: AuthorityList,
// }

// impl TestApi {
// 	pub fn new(genesis_authorities: AuthorityList) -> Self {
// 		TestApi { genesis_authorities }
// 	}
// }

// pub(crate) struct RuntimeApi {
// 	inner: TestApi,
// }

// impl ProvideRuntimeApi<Block> for TestApi {
// 	type Api = RuntimeApi;

// 	fn runtime_api(&self) -> ApiRef<'_, Self::Api> {
// 		RuntimeApi { inner: self.clone() }.into()
// 	}
// }

// impl GenesisAuthoritySetProvider<Block> for TestApi {
// 	fn get(&self) -> sp_blockchain::Result<AuthorityList> {
// 		Ok(self.genesis_authorities.clone())
// 	}
// }


// #[test]
// fn testx(){
    
// }