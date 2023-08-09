use crate::std::collections::BTreeMap;

use super::*;

#[derive(Debug)]
struct BlockRecord {
    /// Block height
    number: BlockNumber,
    /// Parent hash
    parent: Hash,
}

/// A blockchain structure.
#[derive(Debug)]
pub struct DummyChain {
    inner: BTreeMap<Hash, BlockRecord>,
    finalized: (BlockNumber, Hash),
}

impl DummyChain {
    pub fn new() -> Self {
        let mut inner = BTreeMap::new();
        inner.insert(
            GENESIS_HASH,
            BlockRecord {
                number: 0,
                parent: NULL_HASH,
            },
        );
        DummyChain {
            inner,
            finalized: (0, GENESIS_HASH),
        }
    }

    /// Add a chain to current chain.
    pub fn push_blocks(&mut self, mut parent: Hash, blocks: &[Hash]) {
        if blocks.is_empty() {
            return;
        }

        for i in blocks {
            self.push_block(parent, i);
            parent = i
        }
    }

    /// Add a block to current chain.
    pub fn push_block(&mut self, parent: Hash, block: Hash) {
        let block_number = self.inner.get(parent).unwrap().number + 1;
        self.inner.insert(
            block,
            BlockRecord {
                number: block_number,
                parent,
            },
        );
    }

    pub fn last_finalized(&self) -> (BlockNumber, Hash) {
        self.finalized
    }

    /// Get block after the last finalized block
    pub fn next_to_be_finalized(&self) -> Result<(BlockNumber, Hash), ()> {
        for (hash, record) in self.inner.iter().rev() {
            if record.number == self.finalized.0 + 1 {
                return Ok((record.number, hash.clone()));
            }
        }

        Err(())
    }

    /// Finalized a block.
    pub fn finalize_block(&mut self, block: Hash) -> bool {
        #[cfg(feature = "std")]
        log::trace!("finalize_block: {:?}", block);
        if let Some(b) = self.inner.get(&block) {
            #[cfg(feature = "std")]
            log::trace!("finalize block: {:?}", b);
            if self
                .inner
                .get(&b.parent)
                .map(|p| p.number < b.number)
                .unwrap_or(false)
            {
                self.finalized = (b.number, block);
                #[cfg(feature = "std")]
                log::info!("new finalized = {:?}", self.finalized);
                return true;
            } else {
                panic!("block {} is not a descendent of {}", b.number, b.parent);
            }
        }

        false
    }
}
