// Hotstuff block store, just a easy key-value store.
use std::marker::PhantomData;
use std::sync::Arc;

use sc_client_api::Backend;
use sp_runtime::traits::Block as BlockT;

use crate::client::ClientForHotstuff;

use sp_blockchain::Error;

pub struct Store<Block: BlockT, BE: Backend<Block>, C: ClientForHotstuff<Block, BE>> {
	backend: Arc<C>,
	_p1: PhantomData<Block>,
	_p2: PhantomData<BE>,
}

impl<Block: BlockT, BE: Backend<Block>, C: ClientForHotstuff<Block, BE>> Store<Block, BE, C> {
	pub fn new(backend: Arc<C>) -> Self {
		Self { backend, _p1: PhantomData, _p2: PhantomData }
	}

	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		self.backend.get_aux(key)
	}

	pub fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), Error> {
		self.backend.insert_aux(&[(key, value)], &[])
	}
}

#[cfg(test)]
mod tests{

	use super::*;

	#[test]
	fn test_create_store_with_substrate_client_should_work(){
		let client = substrate_test_runtime_client::new();
		let mut store = Store::new(Arc::new(client));

		store.set(b"key0", b"value0").unwrap();

		assert_eq!(store.get(b"key0").unwrap().unwrap(), b"value0");
	}	
}

