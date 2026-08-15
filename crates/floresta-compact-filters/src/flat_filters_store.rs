// SPDX-License-Identifier: MIT OR Apache-2.0

use std::convert::TryFrom;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use crate::IterableFilterStore;
use crate::IterableFilterStoreError;

/// The maximum size that a block filter can have.
pub const MAX_FILTER_SIZE: u32 = 1_000_000;

pub struct FiltersIterator {
    reader: BufReader<File>,
}

impl Iterator for FiltersIterator {
    type Item = (u32, crate::bip158::BlockFilter);

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = [0; 4];

        self.reader.read_exact(&mut buf).ok()?;
        let height = u32::from_le_bytes(buf);

        self.reader.read_exact(&mut buf).ok()?;
        let length = u32::from_le_bytes(buf);

        debug_assert!(
            length < 1_000_000,
            "filter for block {} has length {}",
            height,
            length,
        );

        let mut buf = vec![0_u8; length as usize];
        self.reader.read_exact(&mut buf).ok()?;
        let filter = crate::bip158::BlockFilter::new(&buf);

        Some((height, filter))
    }
}

struct FlatFiltersStoreInner {
    file: std::fs::File,
    index: std::fs::File,
    path: PathBuf,
}

impl From<PoisonError<MutexGuard<'_, FlatFiltersStoreInner>>> for IterableFilterStoreError {
    fn from(_: PoisonError<MutexGuard<'_, FlatFiltersStoreInner>>) -> Self {
        Self::PoisonedLock
    }
}

pub struct FlatFiltersStore(Mutex<FlatFiltersStoreInner>);

impl FlatFiltersStore {
    /// Opens (creating if needed) the filter file and its companion index at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`IterableFilterStoreError::Io`] if either file cannot be opened, or if the
    /// index header cannot be written.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, IterableFilterStoreError> {
        let path = path.as_ref();

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let mut index_path = path.as_os_str().to_owned();
        index_path.push("-index");
        let mut index = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&index_path)?;

        index.seek(SeekFrom::Start(0))?;
        index.write_all(&4_u64.to_le_bytes())?;

        Ok(Self(Mutex::new(FlatFiltersStoreInner {
            file,
            path: path.into(),
            index,
        })))
    }
}

impl TryFrom<&PathBuf> for FlatFiltersStore {
    type Error = std::io::Error;

    fn try_from(path: &PathBuf) -> Result<Self, Self::Error> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let index = format!("{}-index", path.to_string_lossy());
        let mut index = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(index)?;

        index.seek(SeekFrom::Start(0))?;
        index.write_all(&4_u64.to_le_bytes())?;

        Ok(Self(Mutex::new(FlatFiltersStoreInner {
            file,
            index,
            path: path.clone(),
        })))
    }
}

impl IntoIterator for FlatFiltersStore {
    type Item = (u32, crate::bip158::BlockFilter);
    type IntoIter = FiltersIterator;

    /// # Panics
    ///
    /// Panics if the store lock is poisoned, or if the underlying file cannot be sought or
    /// cloned. [`IntoIterator::into_iter`] returns the iterator directly, so there is no way
    /// to report these as errors; use [`IterableFilterStore::iter`] for a fallible version.
    #[allow(
        clippy::unwrap_used,
        reason = "IntoIterator cannot return a Result; the panic is documented above and \
                  IterableFilterStore::iter offers the fallible path"
    )]
    fn into_iter(self) -> Self::IntoIter {
        let mut inner = self.0.lock().unwrap();
        inner.file.seek(SeekFrom::Start(4)).unwrap();
        let reader = BufReader::new(inner.file.try_clone().unwrap());
        FiltersIterator { reader }
    }
}

impl IterableFilterStore for FlatFiltersStore {
    type I = FiltersIterator;
    fn set_height(&self, height: u32) -> Result<(), IterableFilterStoreError> {
        let mut inner = self.0.lock()?;
        inner.file.seek(SeekFrom::Start(0))?;
        inner.file.write_all(&height.to_le_bytes())?;

        Ok(())
    }

    fn get_height(&self) -> Result<u32, IterableFilterStoreError> {
        let mut inner = self.0.lock()?;

        let mut buf = [0; 4];
        inner.file.seek(SeekFrom::Start(0))?;
        inner.file.read_exact(&mut buf)?;

        Ok(u32::from_le_bytes(buf))
    }

    fn iter(&self, start_height: Option<usize>) -> Result<Self::I, IterableFilterStoreError> {
        let mut inner = self.0.lock()?;
        let new_file = File::open(inner.path.clone())?;
        let mut reader = BufReader::new(new_file);

        let start_height = start_height.unwrap_or(0) as u32;

        #[allow(clippy::arithmetic_side_effects, reason = "invariant above")]
        let index = {
            let start_height = start_height - (start_height % 50_000);

            // take the index by dividing by 50_000
            (start_height / 50_000) * 8
        };

        // seek to the index
        inner.index.seek(SeekFrom::Start(index as u64))?;

        // read the position of the file
        let mut buf = [0; 8];
        inner.index.read_exact(&mut buf)?;
        let pos = u64::from_le_bytes(buf);

        // seek to the position
        reader.seek(SeekFrom::Start(pos))?;
        Ok(FiltersIterator { reader })
    }

    fn put_filter(
        &self,
        block_filter: crate::bip158::BlockFilter,
        height: u32,
    ) -> Result<(), IterableFilterStoreError> {
        let length = block_filter.content.len() as u32;

        if length > MAX_FILTER_SIZE {
            return Err(IterableFilterStoreError::OversizedBlockFilter);
        }

        let mut inner = self.0.lock()?;

        let offset = inner.file.seek(SeekFrom::End(0))?;
        // save the position of the file for every 50_000 blocks, so we can
        // start the rescan from a given height
        if height % 50_000 == 0 {
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "height / 50_000 is at most u32::MAX / 50_000, so scaling by the \
                          8-byte index stride cannot overflow"
            )]
            let byte_offset = (height / 50_000) * 8;

            inner.index.seek(SeekFrom::Start(byte_offset as u64))?;
            inner.index.write_all(&offset.to_le_bytes())?;
        }

        inner.file.write_all(&height.to_le_bytes())?;
        inner.file.write_all(&length.to_le_bytes())?;
        inner.file.write_all(&block_filter.content)?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm,
    reason = "test code: a panic is the assertion failing, which is the intent"
)]
mod tests {
    use std::collections::HashMap;
    use std::fs::remove_file;
    use std::sync::Arc;

    use bitcoin::Block;
    use bitcoin::BlockHash;
    use bitcoin::OutPoint;
    use bitcoin::Transaction;
    use bitcoin::Txid;
    use bitcoin::Work;
    use bitcoin::block::Header as BlockHeader;
    use bitcoin::hashes::sha256;
    use floresta_chain::BlockConsumer;
    use floresta_chain::BlockchainError;
    use floresta_chain::UtxoData;
    use floresta_chain::pruned_utreexo::BlockchainInterface;
    use floresta_chain::pruned_utreexo::IBDState;
    use rustreexo::proof::Proof;
    use rustreexo::stump::Stump;

    use super::FlatFiltersStore;
    use super::MAX_FILTER_SIZE;
    use crate::IterableFilterStore;
    use crate::IterableFilterStoreError;
    use crate::bip158::BlockFilter;
    use crate::network_filters::NetworkFilters;

    /// A filesystem path unique to this test run.
    ///
    /// The filter store writes real files next to the crate root, and the suite is executed
    /// once per feature combination, so a fixed name can collide with a previous run's
    /// leftovers. Deriving the name from the test plus a counter keeps each case isolated.
    fn unique_path(tag: &str) -> String {
        use std::sync::atomic::AtomicU64;
        use std::sync::atomic::Ordering;

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();

        format!("test-filters-{tag}-{pid}-{n}")
    }

    #[test]
    fn test_filter_store() {
        let path = unique_path("filter-store");
        let store = FlatFiltersStore::new(&path).unwrap();

        let res = store.get_height().unwrap_err();
        assert!(matches!(res, crate::IterableFilterStoreError::Io(_)));
        store.set_height(1).expect("could not set height");
        assert_eq!(store.get_height().unwrap(), 1);

        let filter = BlockFilter::new(&[10, 11, 12, 13]);
        store
            .put_filter(filter.clone(), 1)
            .expect("could not put filter");

        let mut iter = store.iter(Some(0)).expect("could not get iterator");
        assert_eq!((1, filter), iter.next().unwrap());

        assert_eq!(iter.next(), None);
        remove_file(&path).expect("could not remove file after test");
        remove_file(format!("{path}-index")).expect("could not remove index after test");
    }

    /// A filter larger than [`MAX_FILTER_SIZE`] must be rejected with a variant the caller
    /// can branch on, rather than being written to disk.
    #[test]
    fn propagates_oversized_block_filter() {
        let path = unique_path("oversized");
        let store = FlatFiltersStore::new(&path).unwrap();

        let oversized = BlockFilter::new(&vec![0_u8; (MAX_FILTER_SIZE as usize) + 1]);
        let err = store.put_filter(oversized, 1).unwrap_err();

        assert!(matches!(
            err,
            IterableFilterStoreError::OversizedBlockFilter
        ));

        remove_file(&path).expect("could not remove file after test");
        remove_file(format!("{path}-index")).expect("could not remove index after test");
    }

    /// A thread panicking while holding the store lock poisons it; every later access must
    /// report [`IterableFilterStoreError::PoisonedLock`] instead of panicking again.
    #[test]
    fn propagates_poisoned_lock() {
        let path = unique_path("poisoned");
        let store = Arc::new(FlatFiltersStore::new(&path).unwrap());
        let poisoner = Arc::clone(&store);

        let handle = std::thread::spawn(move || {
            let _guard = poisoner.0.lock().unwrap();
            panic!("poisoning the lock on purpose");
        });
        assert!(handle.join().is_err());

        let err = store.set_height(1).unwrap_err();
        assert!(matches!(err, IterableFilterStoreError::PoisonedLock));

        remove_file(&path).expect("could not remove file after test");
        remove_file(format!("{path}-index")).expect("could not remove index after test");
    }

    /// `match_any` walks stored filters and asks the chain for each block hash. When the
    /// chain doesn't know a height, that must surface as
    /// [`IterableFilterStoreError::BlockNotFound`] carrying the height and the chain's own
    /// message, rather than being discarded or unwrapped.
    #[test]
    fn propagates_block_not_found_from_chain() {
        let path = unique_path("block-not-found");
        let store = FlatFiltersStore::new(&path).unwrap();
        store.set_height(1).unwrap();
        store
            .put_filter(BlockFilter::new(&[1, 2, 3, 4]), 1)
            .unwrap();

        let filters = NetworkFilters::new(store).unwrap();
        let err = filters
            .match_any(vec![&[1, 2, 3, 4]], Some(0), None, ChainWithoutBlocks)
            .unwrap_err();

        match err {
            IterableFilterStoreError::BlockNotFound { height, source } => {
                assert_eq!(height, 1);
                assert!(
                    !source.is_empty(),
                    "the chain's own message must be preserved"
                );
            }
            other => panic!("expected BlockNotFound, got {other:?}"),
        }

        remove_file(&path).expect("could not remove file after test");
        remove_file(format!("{path}-index")).expect("could not remove index after test");
    }

    /// A chain backend that knows about no blocks at all, used to drive the failure path in
    /// `match_any`. Everything the test doesn't exercise is left unimplemented on purpose.
    #[derive(Debug)]
    struct ChainWithoutBlocks;

    #[derive(Debug)]
    struct NoSuchBlock;

    impl std::fmt::Display for NoSuchBlock {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "this chain has no blocks")
        }
    }

    impl std::error::Error for NoSuchBlock {}

    impl BlockchainInterface for ChainWithoutBlocks {
        type Error = NoSuchBlock;

        fn get_block_hash(&self, _height: u32) -> Result<BlockHash, Self::Error> {
            Err(NoSuchBlock)
        }

        fn get_tx(&self, _txid: &Txid) -> Result<Option<Transaction>, Self::Error> {
            unimplemented!()
        }
        fn get_height(&self) -> Result<u32, Self::Error> {
            unimplemented!()
        }
        fn estimate_fee(&self, _target: usize) -> Result<f64, Self::Error> {
            unimplemented!()
        }
        fn get_block(&self, _hash: &BlockHash) -> Result<Block, Self::Error> {
            unimplemented!()
        }
        fn get_best_block(&self) -> Result<(u32, BlockHash), Self::Error> {
            unimplemented!()
        }
        fn get_block_header(&self, _hash: &BlockHash) -> Result<BlockHeader, Self::Error> {
            unimplemented!()
        }
        fn subscribe(&self, _tx: Arc<dyn BlockConsumer>) {
            unimplemented!()
        }
        fn is_in_ibd(&self) -> bool {
            unimplemented!()
        }
        fn is_coinbase_mature(&self, _h: u32, _b: BlockHash) -> Result<bool, Self::Error> {
            unimplemented!()
        }
        fn get_block_locator(&self) -> Result<Vec<BlockHash>, Self::Error> {
            unimplemented!()
        }
        fn get_block_locator_for_tip(
            &self,
            _tip: BlockHash,
        ) -> Result<Vec<BlockHash>, BlockchainError> {
            unimplemented!()
        }
        fn get_validation_index(&self) -> Result<u32, Self::Error> {
            unimplemented!()
        }
        fn get_block_height(&self, _hash: &BlockHash) -> Result<Option<u32>, Self::Error> {
            unimplemented!()
        }
        fn update_acc(
            &self,
            _acc: Stump,
            _block: &Block,
            _height: u32,
            _proof: Proof,
            _del_hashes: Vec<sha256::Hash>,
        ) -> Result<Stump, Self::Error> {
            unimplemented!()
        }
        fn get_chain_tips(&self) -> Result<Vec<BlockHash>, Self::Error> {
            unimplemented!()
        }
        fn validate_block(
            &self,
            _block: &Block,
            _proof: Proof,
            _inputs: HashMap<OutPoint, UtxoData>,
            _del_hashes: Vec<sha256::Hash>,
            _acc: Stump,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        fn get_fork_point(&self, _block: BlockHash) -> Result<BlockHash, Self::Error> {
            unimplemented!()
        }
        fn get_params(&self) -> bitcoin::params::Params {
            unimplemented!()
        }
        fn acc(&self) -> Stump {
            unimplemented!()
        }
        fn get_work(&self, _tip: BlockHash) -> Result<Work, Self::Error> {
            unimplemented!()
        }
        fn size_on_disk(&self) -> Result<u64, Self::Error> {
            unimplemented!()
        }
        fn ibd_state(&self) -> IBDState {
            unimplemented!()
        }
    }
}
