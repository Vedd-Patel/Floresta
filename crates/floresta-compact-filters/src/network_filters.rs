// SPDX-License-Identifier: MIT OR Apache-2.0

use bitcoin::BlockHash;
use bitcoin::bip158::BlockFilter;
use floresta_chain::pruned_utreexo::BlockchainInterface;

use crate::IterableFilterStore;
use crate::IterableFilterStoreError;

#[derive(Debug)]
pub struct NetworkFilters<Storage: IterableFilterStore> {
    filters: Storage,
}

impl<Storage: IterableFilterStore> NetworkFilters<Storage> {
    /// Wraps a filter store, initialising its height if it has none yet.
    ///
    /// # Errors
    ///
    /// Returns whichever [`IterableFilterStoreError`] the store reports when the initial
    /// height cannot be written.
    pub fn new(filters: Storage) -> Result<Self, IterableFilterStoreError> {
        if filters.get_height().is_err() {
            filters.set_height(0)?;
        }

        Ok(Self { filters })
    }

    pub fn match_any(
        &self,
        query: Vec<&[u8]>,
        start_height: Option<u32>,
        stop_height: Option<u32>,
        chain: impl BlockchainInterface,
    ) -> Result<Vec<BlockHash>, IterableFilterStoreError> {
        let mut blocks = Vec::new();
        let iter = query.into_iter();

        let start_height = start_height.map(|n| n as usize);

        for (height, filter) in self.filters.iter(start_height)? {
            let hash = match chain.get_block_hash(height) {
                Ok(hash) => hash,
                Err(e) => {
                    return Err(IterableFilterStoreError::BlockNotFound {
                        height,
                        source: e.to_string(),
                    });
                }
            };

            if filter.match_any(&hash, &mut iter.clone())? {
                blocks.push(hash);
            }

            if let Some(stop_at) = stop_height {
                if height >= stop_at {
                    break;
                };
            }
        }
        Ok(blocks)
    }

    pub fn push_filter(
        &self,
        filter: BlockFilter,
        height: u32,
    ) -> Result<(), IterableFilterStoreError> {
        self.filters.put_filter(filter, height)
    }

    pub fn get_height(&self) -> Result<u32, IterableFilterStoreError> {
        self.filters.get_height()
    }

    pub fn save_height(&self, height: u32) -> Result<(), IterableFilterStoreError> {
        self.filters.set_height(height)
    }
}
