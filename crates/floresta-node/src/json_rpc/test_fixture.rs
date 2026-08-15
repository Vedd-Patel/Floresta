// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared test scaffolding for the JSON-RPC endpoint modules.
//!
//! Building an [`RpcImpl`] by hand needs a chainstate, a watch-only wallet, a node handle
//! and a handful of runtime knobs. Every endpoint module needs the same thing to exercise
//! its error paths, so it is assembled once here rather than in each test module.
//!
//! The node handle is backed by a channel whose receiver is kept alive by the fixture, so
//! requests that reach the node queue up instead of failing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test support: a panic here is a broken fixture, which should fail the test"
)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use bitcoin::Network;
use floresta_chain::AssumeValidArg;
use floresta_chain::ChainState;
use floresta_chain::FlatChainStore;
use floresta_chain::FlatChainStoreConfig;
use floresta_watch_only::AddressCache;
use floresta_watch_only::kv_database::KvDatabase;
use floresta_wire::node::NodeNotification;
use floresta_wire::node_handle::NodeHandle;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::unbounded_channel;

use crate::json_rpc::server::RpcImpl;

/// Size of the test chainstore's index and header files, in entries.
const TEST_CHAINSTORE_SIZE: usize = 32_768;

/// Size of the test chainstore's fork file, in entries.
const TEST_FORK_FILE_SIZE: usize = 10_000;

/// An [`RpcImpl`] wired up for tests, together with the pieces that must outlive it.
pub(super) struct TestRpc {
    /// The subject under test.
    pub(super) rpc: RpcImpl<Arc<ChainState<FlatChainStore>>>,

    /// Held only to keep the node channel open for the fixture's lifetime; if this were
    /// dropped, every request to the node would fail as unreachable.
    _node_rx: UnboundedReceiver<NodeNotification>,

    /// Temporary directory holding the chainstore and wallet, removed on drop.
    datadir: PathBuf,
}

impl Drop for TestRpc {
    fn drop(&mut self) {
        // Best-effort cleanup; a leftover temp dir must not fail a test.
        let _ = std::fs::remove_dir_all(&self.datadir);
    }
}

/// Builds an [`RpcImpl`] backed by a fresh, empty regtest chainstate and wallet.
///
/// The chain contains only genesis, which is what makes the "not found" and "out of range"
/// error paths reachable without syncing anything.
pub(super) fn test_rpc() -> TestRpc {
    let test_id = rand::random::<u64>();
    let datadir = PathBuf::from(format!("./tmp-db/rpc-{test_id}"));

    let chain_config = FlatChainStoreConfig {
        block_index_size: Some(TEST_CHAINSTORE_SIZE),
        headers_file_size: Some(TEST_CHAINSTORE_SIZE),
        fork_file_size: Some(TEST_FORK_FILE_SIZE),
        cache_size: Some(10),
        file_permission: Some(0o660),
        path: datadir.join("chain"),
    };

    let chainstore = FlatChainStore::new(chain_config).expect("test chainstore");
    let chain = Arc::new(
        ChainState::open(chainstore, Network::Regtest, AssumeValidArg::Disabled)
            .expect("test chainstate"),
    );

    let wallet_db = KvDatabase::new(datadir.join("wallet")).expect("test wallet database");
    let wallet = Arc::new(AddressCache::new(wallet_db).expect("test wallet"));

    let (node_tx, node_rx) = unbounded_channel();

    let rpc = RpcImpl {
        block_filter_storage: None,
        network: Network::Regtest,
        chain,
        wallet,
        node: NodeHandle::new(node_tx),
        kill_signal: Arc::new(RwLock::new(false)),
        inflight: Arc::new(RwLock::new(HashMap::new())),
        log_path: datadir.join("output.log"),
        start_time: Instant::now(),
        user_agent: "/floresta-test/".to_string(),
        proxy: None,
    };

    TestRpc {
        rpc,
        _node_rx: node_rx,
        datadir,
    }
}
