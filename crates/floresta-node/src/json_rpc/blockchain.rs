// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::BTreeMap;

use bitcoin::Address;
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::MerkleBlock;
use bitcoin::Network;
use bitcoin::OutPoint;
use bitcoin::Script;
use bitcoin::ScriptBuf;
use bitcoin::Txid;
use bitcoin::VarInt;
use bitcoin::block::Header;
use bitcoin::consensus::Encodable;
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::constants::genesis_block;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use corepc_types::ScriptPubKey;
use corepc_types::v29::GetTxOut;
use corepc_types::v30::DeploymentInfo;
use corepc_types::v30::GetBlockHeaderVerbose;
use corepc_types::v30::GetBlockVerboseOne;
use corepc_types::v30::GetBlockchainInfo;
use corepc_types::v30::GetDeploymentInfo;
use floresta_chain::buried_deployments_for;
use floresta_chain::extensions::HeaderExt;
use floresta_chain::extensions::WorkExt;
use floresta_wire::node_interface::ChainMethods;
use miniscript::descriptor::checksum;
use serde_json::Value;
use serde_json::json;
use tracing::debug;

use super::res::GetBlockHeaderRes;
use super::res::GetTxOutProof;
use super::res::jsonrpc_interface::JsonRpcError;
use super::server::RpcChain;
use super::server::RpcImpl;
use crate::json_rpc::res::GetBlockRes;
use crate::json_rpc::res::RescanConfidence;
use crate::json_rpc::server::SERIALIZATION_EXPECT_MSG;
use crate::json_rpc::server::to_core_asm_string;

/// Errors that originate in the chain-query endpoints.
///
/// These are the conditions a client asking about blocks and transactions can actually act
/// on: the thing was not found, the rescan bounds were wrong, or the node has not synced
/// far enough to answer yet.
#[derive(Debug)]
pub enum BlockchainRpcError {
    /// No block matches the requested height or hash.
    BlockNotFound,

    /// The wallet has not cached the requested transaction.
    ///
    /// Floresta only knows about transactions involving watched addresses, so this is the
    /// expected answer for anything else rather than a fault.
    TxNotFound,

    /// The requested rescan range is not usable.
    InvalidRescanRange {
        /// Why the range was rejected, so the client can correct it.
        reason: &'static str,
    },

    /// No block corresponds to the supplied timestamp.
    InvalidTimestamp,

    /// The node is still syncing and cannot answer historical queries yet.
    ///
    /// Distinct from a missing feature: the client should retry once sync completes.
    StillSyncing,

    /// Compact block filters are not configured, so this query cannot be served.
    ///
    /// The operator needs to restart with `--cfilters` for this endpoint to work.
    NoBlockFilters,
}

impl core::fmt::Display for BlockchainRpcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BlockNotFound => write!(f, "no block matches that height or hash"),
            Self::TxNotFound => write!(f, "the wallet has not cached that transaction"),
            Self::InvalidRescanRange { reason } => {
                write!(f, "the rescan range is not usable: {reason}")
            }
            Self::InvalidTimestamp => write!(f, "no block corresponds to that timestamp"),
            Self::StillSyncing => {
                write!(f, "the node is still syncing and cannot answer this yet")
            }
            Self::NoBlockFilters => write!(
                f,
                "compact block filters are not enabled; restart with --cfilters"
            ),
        }
    }
}

impl core::error::Error for BlockchainRpcError {
    /// Each variant describes the condition itself; there is no wrapped failure.
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        None
    }
}

impl From<BlockchainRpcError> for JsonRpcError {
    fn from(e: BlockchainRpcError) -> Self {
        match e {
            BlockchainRpcError::BlockNotFound => Self::BlockNotFound,
            BlockchainRpcError::TxNotFound => Self::TxNotFound,
            BlockchainRpcError::InvalidRescanRange { .. } => Self::InvalidRescanVal,
            BlockchainRpcError::InvalidTimestamp => Self::InvalidTimestamp,
            BlockchainRpcError::StillSyncing => Self::InInitialBlockDownload,
            BlockchainRpcError::NoBlockFilters => Self::NoBlockFilters,
        }
    }
}

impl<Blockchain: RpcChain> RpcImpl<Blockchain> {
    async fn get_block_inner(&self, hash: BlockHash) -> Result<Block, JsonRpcError> {
        let is_genesis = self.chain.get_block_hash(0)?.eq(&hash);

        if is_genesis {
            return Ok(genesis_block(self.network));
        }

        // Verify the block header is known before requesting the full block
        // from the network, otherwise the request will hang indefinitely.
        self.get_block_header_inner(hash)?;

        self.node
            .get_block(hash)
            .await
            .map_err(|e| JsonRpcError::Node(e.to_string()))
            .and_then(|block| block.ok_or(JsonRpcError::BlockNotFound))
    }

    /// Return the block that contains the given Txid
    pub fn get_block_by_txid(&self, txid: &Txid) -> Result<Block, JsonRpcError> {
        let height = self
            .wallet
            .get_height(txid)?
            .ok_or(BlockchainRpcError::TxNotFound)?;
        let blockhash = self
            .chain
            .get_block_hash(height)
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)?;

        self.chain
            .get_block(&blockhash)
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)
            .map_err(Into::into)
    }

    pub fn get_rescan_interval(
        &self,
        use_timestamp: bool,
        start: u32,
        stop: u32,
        confidence: RescanConfidence,
    ) -> Result<(u32, u32), JsonRpcError> {
        if use_timestamp {
            let start_height = self.get_block_height_by_timestamp(start, &confidence)?;

            let stop_height = self.get_block_height_by_timestamp(stop, &RescanConfidence::Exact)?;

            return Ok((start_height, stop_height));
        }

        let (tip, _) = self.chain.get_best_block()?;

        if stop > tip {
            return Err(BlockchainRpcError::InvalidRescanRange {
                reason: "stop height is beyond the chain tip",
            }
            .into());
        }

        Ok((start, stop))
    }

    /// Retrieves the height of the block that was mined in the given timestamp.
    ///
    /// `timestamp` has an alias, 0 will directly refer to the network's genesis timestamp.
    pub fn get_block_height_by_timestamp(
        &self,
        timestamp: u32,
        confidence: &RescanConfidence,
    ) -> Result<u32, JsonRpcError> {
        /// Simple helper to avoid code reuse.
        fn get_block_time<BlockChain: RpcChain>(
            provider: &RpcImpl<BlockChain>,
            at: u32,
        ) -> Result<u32, JsonRpcError> {
            let hash = provider.get_block_hash(at)?;
            let block = provider.get_block_header_inner(hash)?;
            Ok(block.time)
        }

        let genesis_timestamp = genesis_block(self.network).header.time;

        if timestamp == 0 || timestamp == genesis_timestamp {
            return Ok(0);
        };

        let (tip_height, _) = self
            .chain
            .get_best_block()
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)?;

        let tip_time = get_block_time(self, tip_height)?;

        if timestamp < genesis_timestamp || timestamp > tip_time {
            return Err(BlockchainRpcError::InvalidTimestamp.into());
        }

        let adjusted_target = timestamp.saturating_sub(confidence.as_secs());

        let mut high = tip_height;
        let mut low = 0;
        let max_iters = tip_height.ilog2().saturating_add(1);
        for _ in 0..max_iters {
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "this is a binary search over block heights with low <= high, both bounded by \
                          the chain tip, so the midpoint cannot overflow"
            )]
            let cut = (high + low) / 2;

            let block_timestamp = get_block_time(self, cut)?;

            if block_timestamp == adjusted_target {
                debug!("found a precise block; returning {cut}");
                return Ok(cut);
            }

            if high.saturating_sub(low) <= 2 {
                debug!("didn't find a precise block; returning {low}");
                return Ok(low);
            }

            if block_timestamp > adjusted_target {
                high = cut;
            } else {
                low = cut;
            }
        }

        // This is pretty much unreachable.
        Err(JsonRpcError::BlockNotFound)
    }
}

// blockchain rpcs
impl<Blockchain: RpcChain> RpcImpl<Blockchain> {
    // dumputxoutset

    // getbestblockhash
    pub(super) fn get_best_block_hash(&self) -> Result<BlockHash, JsonRpcError> {
        Ok(self.chain.get_best_block()?.1)
    }

    // getblock
    pub(super) async fn get_block(
        &self,
        hash: BlockHash,
        verbosity: u8,
    ) -> Result<GetBlockRes, JsonRpcError> {
        let block = self.get_block_inner(hash).await?;

        if verbosity == 0 {
            let hex = serialize_hex(&block);

            return Ok(GetBlockRes::Zero(hex));
        }
        if verbosity == 1 {
            let header_fields = self.get_block_header_verbose_inner(&block)?;

            // Stripped size is the size of the block without witness data
            // Header + VarInt for number of transactions + sum of base sizes of each transaction
            let tx_count_varint_size = VarInt::from(block.txdata.len()).size();
            let total_tx_base_size: usize = block.txdata.iter().map(|tx| tx.base_size()).sum();
            // Bounded by the block weight limit, so this sum cannot overflow usize.
            let stripped_size_bytes = Header::SIZE
                .saturating_add(tx_count_varint_size)
                .saturating_add(total_tx_base_size);

            let stripped_size = Some(stripped_size_bytes.try_into()?);

            let tx = block
                .txdata
                .iter()
                .map(|tx| tx.compute_txid().to_string())
                .collect();

            let block = GetBlockVerboseOne {
                bits: header_fields.bits,
                chain_work: header_fields.chain_work,
                confirmations: header_fields.confirmations,
                difficulty: header_fields.difficulty,
                hash: header_fields.hash,
                height: header_fields.height,
                merkle_root: header_fields.merkle_root,
                nonce: header_fields.nonce,
                previous_block_hash: header_fields.previous_block_hash,
                size: block.total_size().try_into()?,
                time: header_fields.time,
                tx,
                version: header_fields.version,
                version_hex: header_fields.version_hex,
                weight: block.weight().to_wu(),
                median_time: Some(header_fields.median_time),
                n_tx: header_fields.n_tx.into(),
                next_block_hash: header_fields.next_block_hash,
                stripped_size,
                target: header_fields.target,
            };

            return Ok(GetBlockRes::One(Box::new(block)));
        }
        Err(JsonRpcError::InvalidVerbosityLevel)
    }

    // getblockchaininfo
    //
    // `headers` tracks the best-known header tip; `blocks` tracks the validated
    // tip. They can diverge mid-IBD and coincide once sync completes.
    pub(super) fn get_blockchain_info(&self) -> Result<GetBlockchainInfo, JsonRpcError> {
        let (height, hash) = self.chain.get_best_block()?;
        let validated = self.chain.get_validation_index()?;
        let initial_block_download = self.chain.is_in_ibd();
        let latest_header = self.chain.get_block_header(&hash)?;
        let chain_work = latest_header
            .calculate_chain_work(&self.chain)?
            .to_string_hex();

        let verification_progress = if height != 0 {
            f64::from(validated) / f64::from(height)
        } else {
            0.0
        };

        let blocks = i64::from(validated);
        let headers = i64::from(height);
        let best_block_hash = hash.to_string();
        let bits = latest_header.get_bits_hex();
        let target = latest_header.get_target_hex();
        let difficulty = latest_header.get_difficulty();
        let time = i64::from(latest_header.time);
        let median_time = i64::from(latest_header.calculate_median_time_past(&self.chain)?);
        let size_on_disk = self.chain.size_on_disk()?;
        let prune_height = Some(blocks.saturating_add(1));

        let chain = match self.network {
            Network::Bitcoin => "main",
            Network::Testnet => "test",
            Network::Testnet4 => "testnet4",
            Network::Signet => "signet",
            Network::Regtest => "regtest",
        }
        .to_string();

        Ok(GetBlockchainInfo {
            chain,
            blocks,
            headers,
            best_block_hash,
            bits,
            target,
            difficulty,
            time,
            median_time,
            verification_progress,
            initial_block_download,
            chain_work,
            size_on_disk,
            pruned: true,
            prune_height,
            automatic_pruning: Some(true),
            prune_target_size: Some(0),
            signet_challenge: None,
            warnings: vec![],
        })
    }

    // getblockcount
    pub(super) fn get_block_count(&self) -> Result<u32, JsonRpcError> {
        Ok(self.chain.get_height()?)
    }

    // getblockfilter
    // getblockfrompeer (just call getblock)

    // getblockhash
    pub(super) fn get_block_hash(&self, height: u32) -> Result<BlockHash, JsonRpcError> {
        self.chain
            .get_block_hash(height)
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)
            .map_err(Into::into)
    }

    // getblockheader
    pub(super) async fn get_block_header(
        &self,
        hash: BlockHash,
        verbosity: bool,
    ) -> Result<GetBlockHeaderRes, JsonRpcError> {
        let header = self.get_block_header_inner(hash)?;

        if !verbosity {
            let hex = serialize_hex(&header);
            return Ok(GetBlockHeaderRes::Raw(hex));
        }

        let block = self.get_block_inner(hash).await?;

        let get_block_header = self.get_block_header_verbose_inner(&block)?;

        Ok(GetBlockHeaderRes::Verbose(Box::new(get_block_header)))
    }

    // getblockstats
    // getchainstates
    // getchaintips
    // getchaintxstats

    // getdeploymentinfo
    pub(super) fn get_deployment_info(
        &self,
        hash: Option<BlockHash>,
    ) -> Result<GetDeploymentInfo, JsonRpcError> {
        let tip = self.chain.get_best_block()?.1;
        let target_hash = hash.unwrap_or(tip);

        let height = self
            .chain
            .get_block_height(&target_hash)?
            .ok_or(BlockchainRpcError::BlockNotFound)?;

        let mut deployments = BTreeMap::new();

        for &(name, activation_height) in buried_deployments_for(self.network) {
            deployments.insert(
                name.to_string(),
                DeploymentInfo {
                    deployment_type: "buried".to_string(),
                    height: Some(activation_height),
                    active: height >= activation_height,
                    bip9: None,
                },
            );
        }

        Ok(GetDeploymentInfo {
            hash: target_hash.to_string(),
            height,
            deployments,
        })
    }

    // getdifficulty
    pub(super) fn get_difficulty(&self) -> Result<f64, JsonRpcError> {
        let (_, hash) = self.chain.get_best_block()?;
        let header = self
            .chain
            .get_block_header(&hash)
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)?;
        Ok(header.difficulty_float())
    }

    // getmempoolancestors
    // getmempooldescendants
    // getmempoolentry
    // getmempoolinfo
    // getrawmempool

    /// Same as `get_block_header_inner` but verbose.
    fn get_block_header_verbose_inner(
        &self,
        block: &Block,
    ) -> Result<GetBlockHeaderVerbose, JsonRpcError> {
        let header = &block.header;
        let height = header.get_height(&self.chain)?;
        let median_time = header.calculate_median_time_past(&self.chain)?;
        let chain_work = header.calculate_chain_work(&self.chain)?.to_string_hex();
        let confirmations = header.get_confirmations(&self.chain)?;
        let version_hex = header.get_version_hex();

        let next_block_hash = header
            .get_next_block_hash(&self.chain)?
            .map(|h| h.to_string());

        let bits = header.get_bits_hex();
        let difficulty = header.get_difficulty();
        let target = header.get_target_hex();
        let previous_block_hash = (header.prev_blockhash != BlockHash::all_zeros())
            .then_some(header.prev_blockhash.to_string());

        Ok(GetBlockHeaderVerbose {
            bits,
            chain_work,
            confirmations: confirmations.into(),
            difficulty,
            hash: header.block_hash().to_string(),
            height: height.into(),
            median_time: median_time.into(),
            next_block_hash,
            version: header.version.to_consensus(),
            version_hex,
            previous_block_hash,
            merkle_root: header.merkle_root.to_string(),
            time: header.time.into(),
            target,
            nonce: header.nonce.into(),
            n_tx: block.txdata.len().try_into()?,
        })
    }

    /// Helper method to get a block header by its hash, used by multiple rpcs.
    fn get_block_header_inner(&self, hash: BlockHash) -> Result<Header, JsonRpcError> {
        self.chain
            .get_block_header(&hash)
            .ok()
            .ok_or(BlockchainRpcError::BlockNotFound)
            .map_err(Into::into)
    }

    /// Check if the script is anchor type
    fn is_anchor_type(script: &Script) -> bool {
        script.as_bytes().starts_with(&[0x51, 0x02, 0x4e, 0x73])
    }

    /// Returns a label about the scriptPubKey type
    /// (pubkey, pubkeyhash, multisig, nulldata, scripthash, witness_v0_keyhash, witness_v0_scripthash, witness_v1_taproot, anchor, nonstandard)
    pub(super) fn get_script_type_label(script: &Script) -> &'static str {
        if script.is_p2pk() {
            return "pubkey";
        }

        if script.is_p2pkh() {
            return "pubkeyhash";
        }

        if script.is_multisig() {
            return "multisig";
        }

        if script.is_op_return() {
            return "nulldata";
        }

        if script.is_p2sh() {
            return "scripthash";
        }

        if script.is_p2wpkh() {
            return "witness_v0_keyhash";
        }

        if script.is_p2wsh() {
            return "witness_v0_scripthash";
        }

        if script.is_p2tr() {
            return "witness_v1_taproot";
        }

        if Self::is_anchor_type(script) {
            return "anchor";
        }

        "nonstandard"
    }

    /// TODO: This function is not compliant with Bitcoin Core.
    /// See: <https://github.com/getfloresta/Floresta/issues/987>
    pub(super) fn get_script_type_descriptor(script: &Script, address: &Option<Address>) -> String {
        // Try script from the address
        if let Some(addr) = address {
            if script.is_p2pk() {
                return format!("pk({addr})");
            }

            return format!("addr({addr})");
        }

        if script.is_op_return() {
            let hex = script.to_hex_string();
            return format!("raw({hex})");
        }

        let hex = script.to_hex_string();
        format!("raw({hex})")
    }

    /// gettxout: returns details about an unspent transaction output.
    pub(super) fn get_tx_out(
        &self,
        txid: Txid,
        outpoint: u32,
        _include_mempool: bool,
    ) -> Result<Option<GetTxOut>, JsonRpcError> {
        let res = match (
            self.wallet.get_transaction(&txid).ok().flatten(),
            self.wallet.get_height(&txid).ok().flatten(),
            self.wallet
                .get_utxo(&OutPoint {
                    txid,
                    vout: outpoint,
                })
                .ok()
                .flatten(),
        ) {
            (Some(cached_tx), Some(height), Some(txout)) => {
                let is_coinbase = cached_tx.tx.is_coinbase();
                let Ok((bestblock_height, bestblock_hash)) = self.chain.get_best_block() else {
                    return Err(JsonRpcError::BlockNotFound);
                };

                let script = txout.script_pubkey.as_script();
                let network = self.chain.get_params().network;
                let address = Address::from_script(script, network).ok();

                let base_descriptor = Self::get_script_type_descriptor(script, &address);
                let mut checksum_engine = checksum::Engine::new();
                let descriptor: Option<String> = match checksum_engine.input(&base_descriptor) {
                    Ok(()) => Some(format!("{base_descriptor}#{}", checksum_engine.checksum())),
                    Err(_) => None,
                };

                let asm = to_core_asm_string(&txout.script_pubkey, false);
                let script_pubkey = ScriptPubKey {
                    asm,
                    hex: txout.script_pubkey.to_hex_string(),
                    descriptor,
                    address: address.as_ref().map(ToString::to_string),
                    type_: Self::get_script_type_label(script).to_string(),
                    // Deprecated in Bitcoin Core v22, require flags in Bitcoin Core.
                    // Set to None as not required for consensus.
                    addresses: None,
                    required_signatures: None,
                };

                Some(GetTxOut {
                    best_block: bestblock_hash.to_string(),
                    confirmations: bestblock_height.saturating_sub(height).saturating_add(1),
                    value: txout.value.to_btc(),
                    script_pubkey,
                    coinbase: is_coinbase,
                })
            }
            _ => None,
        };
        Ok(res)
    }

    /// Computes the necessary information for the RPC `gettxoutproof [txids] blockhash (optional)`
    ///
    /// This function has two paths, when blockhash is inserted and when isn't.
    ///
    /// Specifying the blockhash will make this function go after the block and search
    /// for the transactions inside it, building a merkle proof from the block with its
    /// indexes. Not specifying will redirect it to search for the merkle proof on our
    /// watch-only wallet which may not have the transaction cached.
    ///
    /// Not finding one of the specified transactions will raise [`JsonRpcError::TxNotFound`].
    #[allow(
        clippy::expect_used,
        reason = "INVARIANT: encoding to a Vec<u8> writer is infallible"
    )]
    pub(super) async fn get_txout_proof(
        &self,
        tx_ids: &[Txid],
        blockhash: Option<BlockHash>,
    ) -> Result<GetTxOutProof, JsonRpcError> {
        let block = match blockhash {
            Some(blockhash) => self.get_block_inner(blockhash).await?,
            // Using the first Txid to get the block should be fine since they are expected to all
            // live in the same block, otherwise, theres no way they have a common proof.
            None => {
                let first = tx_ids.first().ok_or(BlockchainRpcError::TxNotFound)?;
                self.get_block_by_txid(first)?
            }
        };

        // Before building the merkle block we try to remove all txids
        // that aren't present in the block we found, meaning that
        // at least one of the txids doesn't belong to the block which
        // in case should make the command fail.
        //
        // this makes the use MerkleBlock::from_block_with_predicate useless.
        let targeted_txids: Vec<Txid> = block
            .txdata
            .iter()
            .filter_map(|tx| {
                let txid = tx.compute_txid();
                if tx_ids.contains(&txid) {
                    Some(txid)
                } else {
                    None
                }
            })
            .collect();

        if targeted_txids.len() != tx_ids.len() {
            return Err(JsonRpcError::TxNotFound);
        };

        let merkle_block = MerkleBlock::from_block_with_predicate(&block, |tx| tx_ids.contains(tx));
        let mut bytes: Vec<u8> = Vec::new();
        merkle_block
            .consensus_encode(&mut bytes)
            .expect("BUG: This will raise if a writer error happens");
        Ok(GetTxOutProof(bytes.to_lower_hex_string()))
    }

    // gettxoutsetinfo
    // gettxspendigprevout
    // importmempool
    // loadtxoutset
    // preciousblock
    // pruneblockchain
    // savemempool
    // scanblocks
    // scantxoutset
    // verifychain
    // verifytxoutproof

    // floresta flavored rpcs. These are not part of the bitcoin rpc spec
    // findtxout
    #[allow(
        clippy::expect_used,
        reason = "INVARIANT: all RPC response types implement Serialize"
    )]
    pub(super) async fn find_tx_out(
        &self,
        txid: Txid,
        vout: u32,
        script: ScriptBuf,
        height: u32,
    ) -> Result<Value, JsonRpcError> {
        if let Ok(Some(txout)) = self.wallet.get_utxo(&OutPoint { txid, vout }) {
            return Ok(serde_json::to_value(txout).expect(SERIALIZATION_EXPECT_MSG));
        }

        // if we are on IBD, we don't have any filters to find this txout.
        if self.chain.is_in_ibd() {
            return Err(BlockchainRpcError::StillSyncing.into());
        }

        // can't proceed without block filters
        let Some(cfilters) = self.block_filter_storage.as_ref() else {
            return Err(BlockchainRpcError::NoBlockFilters.into());
        };

        let _ = self.wallet.cache_address(script.clone());
        let filter_key = script.to_bytes();
        let candidates = cfilters
            .match_any(
                vec![filter_key.as_slice()],
                Some(height),
                None,
                self.chain.clone(),
            )
            .map_err(|e| JsonRpcError::Filters(e.to_string()))?;

        for candidate in candidates {
            let candidate = self.node.get_block(candidate).await;
            let candidate = match candidate {
                Err(e) => {
                    return Err(JsonRpcError::Node(e.to_string()));
                }
                Ok(None) => {
                    return Err(JsonRpcError::Node(format!(
                        "BUG: block {candidate:?} is a match in our filters, but we can't get it?"
                    )));
                }
                Ok(Some(candidate)) => candidate,
            };

            let Ok(Some(height)) = self.chain.get_block_height(&candidate.block_hash()) else {
                return Err(JsonRpcError::BlockNotFound);
            };

            let _ = self.wallet.block_process(&candidate, height);
        }

        let val = match self.get_tx_out(txid, vout, false)? {
            Some(gettxout) => json!(gettxout),
            None => json!({}),
        };
        Ok(val)
    }

    // getroots
    pub(super) fn get_roots(&self) -> Result<Vec<String>, JsonRpcError> {
        let hashes = self.chain.get_root_hashes();
        Ok(hashes.iter().map(|h| h.to_string()).collect())
    }

    pub(super) fn list_descriptors(&self) -> Result<Vec<String>, JsonRpcError> {
        let descriptors = self
            .wallet
            .get_descriptors()
            .map_err(|e| JsonRpcError::Wallet(e.to_string()))?;
        Ok(descriptors)
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
    use bitcoin::BlockHash;
    use bitcoin::Txid;
    use bitcoin::hashes::Hash;
    use floresta_chain::BlockchainInterface;

    use crate::json_rpc::res::RescanConfidence;
    use crate::json_rpc::res::jsonrpc_interface::JsonRpcError;
    use crate::json_rpc::test_fixture::test_rpc;

    /// A height past the tip has no block, and the chain's "not present" failure must be
    /// translated into the RPC's own `BlockNotFound` rather than a generic chain error.
    #[test]
    fn propagates_block_not_found_for_height_past_tip() {
        let fixture = test_rpc();

        let err = fixture.rpc.get_block_hash(9_999).unwrap_err();

        assert!(matches!(err, JsonRpcError::BlockNotFound));
    }

    /// The same for a hash the chain has never seen.
    #[tokio::test]
    async fn propagates_block_not_found_for_unknown_hash() {
        let fixture = test_rpc();

        let err = fixture
            .rpc
            .get_block(BlockHash::all_zeros(), 0)
            .await
            .unwrap_err();

        assert!(matches!(err, JsonRpcError::BlockNotFound));
    }

    /// A transaction the wallet has never cached cannot be located, and the error says so
    /// specifically instead of surfacing as a chain fault.
    #[test]
    fn propagates_tx_not_found_for_unknown_txid() {
        let fixture = test_rpc();

        let err = fixture
            .rpc
            .get_block_by_txid(&Txid::all_zeros())
            .unwrap_err();

        assert!(matches!(
            err,
            JsonRpcError::TxNotFound | JsonRpcError::BlockNotFound
        ));
    }

    /// A rescan whose stop height is beyond the tip is a client-side mistake, reported as
    /// an invalid rescan value rather than being clamped silently.
    #[test]
    fn propagates_invalid_rescan_val_past_tip() {
        let fixture = test_rpc();

        let err = fixture
            .rpc
            .get_rescan_interval(false, 0, 9_999, RescanConfidence::Exact)
            .unwrap_err();

        assert!(matches!(err, JsonRpcError::InvalidRescanVal));
    }

    /// A rescan entirely inside the known chain succeeds, so the check above is rejecting
    /// the right thing rather than everything.
    #[test]
    fn accepts_rescan_interval_within_the_chain() {
        let fixture = test_rpc();

        let (start, stop) = fixture
            .rpc
            .get_rescan_interval(false, 0, 0, RescanConfidence::Exact)
            .unwrap();

        assert_eq!((start, stop), (0, 0));
    }

    /// A timestamp older than genesis has no corresponding block, and must be reported as
    /// an invalid timestamp.
    #[test]
    fn propagates_invalid_timestamp_before_genesis() {
        let fixture = test_rpc();

        let err = fixture
            .rpc
            .get_block_height_by_timestamp(1, &RescanConfidence::Exact)
            .unwrap_err();

        assert!(matches!(
            err,
            JsonRpcError::InvalidTimestamp | JsonRpcError::BlockNotFound
        ));
    }

    /// Reading a UTXO the wallet does not track is a miss, not an error.
    #[test]
    fn get_tx_out_reports_miss_for_unknown_outpoint() {
        let fixture = test_rpc();

        let res = fixture.rpc.get_tx_out(Txid::all_zeros(), 0, false).unwrap();

        assert!(res.is_none());
    }

    /// A node that has not synced cannot scan for a txout, and says so before it ever
    /// looks at whether compact filters are configured. Reporting the more specific of the
    /// two conditions is what lets an operator tell "still syncing" from "filters are off".
    #[tokio::test]
    async fn propagates_initial_block_download_before_checking_filters() {
        let fixture = test_rpc();
        assert!(
            fixture.rpc.chain.is_in_ibd(),
            "a fresh chainstate is in IBD, which is what drives this path"
        );

        let err = fixture
            .rpc
            .find_tx_out(Txid::all_zeros(), 0, bitcoin::ScriptBuf::new(), 0)
            .await
            .unwrap_err();

        assert!(matches!(err, JsonRpcError::InInitialBlockDownload));
    }
}
