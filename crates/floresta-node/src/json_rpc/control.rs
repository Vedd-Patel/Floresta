// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use super::res::jsonrpc_interface::JsonRpcError;
use super::server::RpcChain;
use super::server::RpcImpl;

/// Errors that originate in the node-control endpoints.
///
/// These endpoints report on the running process rather than the chain, so their failures
/// are about the request itself, not about node state.
#[derive(Debug)]
pub enum ControlError {
    /// `getmemoryinfo` was called with a mode it does not implement.
    ///
    /// Only `stats` and `mallocinfo` are defined; anything else is a client mistake.
    UnknownMemInfoMode {
        /// What the client asked for, echoed back so the mistake is obvious.
        mode: String,
    },
}

impl core::fmt::Display for ControlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownMemInfoMode { mode } => write!(
                f,
                "unknown getmemoryinfo mode {mode:?}; expected \"stats\" or \"mallocinfo\""
            ),
        }
    }
}

impl core::error::Error for ControlError {
    /// Describes a bad request rather than wrapping a lower-level failure.
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        None
    }
}

impl From<ControlError> for JsonRpcError {
    fn from(e: ControlError) -> Self {
        match e {
            ControlError::UnknownMemInfoMode { .. } => Self::InvalidMemInfoMode,
        }
    }
}

impl<Blockchain: RpcChain> RpcImpl<Blockchain> {
    pub(super) fn get_memory_info(&self, mode: &str) -> Result<GetMemInfoRes, JsonRpcError> {
        #[cfg(target_env = "gnu")]
        match mode {
            "stats" => {
                let info = unsafe { libc::mallinfo() };

                let stats = GetMemInfoStats {
                    locked: MemInfoLocked {
                        used: info.uordblks as u64,
                        free: info.fordblks as u64,
                        total: info.uordblks.saturating_add(info.fordblks) as u64,
                        locked: info.hblkhd as u64,
                        chunks_used: info.ordblks as u64,
                        chunks_free: info.smblks as u64,
                    },
                };

                Ok(GetMemInfoRes::Stats(stats))
            }

            "mallocinfo" => {
                // A XML with the allocator statistics
                let info = unsafe { libc::mallinfo() };
                let info_str = format!(
                    "<malloc version=\"2.0\"><heap nr=\"1\"><allocated>{}</allocated><free>{}</free><total>{}</total><locked>{}</locked><chunks nr=\"{}\"><used>{}</used><free>{}</free></chunks></heap></malloc>",
                    info.hblkhd,
                    info.uordblks,
                    info.fordblks,
                    info.uordblks.saturating_add(info.fordblks),
                    info.hblkhd,
                    info.ordblks,
                    info.smblks,
                );

                Ok(GetMemInfoRes::MallocInfo(info_str))
            }

            _ => Err(ControlError::UnknownMemInfoMode {
                mode: mode.to_string(),
            }
            .into()),
        }

        #[cfg(target_os = "macos")]
        match mode {
            "stats" => {
                let mut info: libc::malloc_statistics_t = unsafe { std::mem::zeroed() };
                unsafe {
                    libc::malloc_zone_statistics(std::ptr::null_mut(), &mut info);
                }

                let stats = GetMemInfoStats {
                    locked: MemInfoLocked {
                        used: info.size_in_use as u64,
                        free: info.size_allocated.saturating_sub(info.size_in_use) as u64,
                        total: info.size_allocated as u64,
                        locked: info.size_allocated as u64,
                        chunks_used: info.blocks_in_use as u64,
                        chunks_free: 0, // Not available on MacOS
                    },
                };

                Ok(GetMemInfoRes::Stats(stats))
            }
            "mallocinfo" => {
                // A XML with the allocator statistics
                let mut info: libc::malloc_statistics_t = unsafe { std::mem::zeroed() };
                unsafe {
                    libc::malloc_zone_statistics(std::ptr::null_mut(), &mut info);
                }

                let info_str = format!(
                    "<malloc version=\"2.0\"><heap nr=\"1\"><allocated>{}</allocated><free>{}</free><total>{}</total><locked>{}</locked><chunks nr=\"{}\"><used>{}</used><free>{}</free></chunks></heap></malloc>",
                    info.size_allocated,
                    info.size_in_use,
                    info.size_allocated.saturating_sub(info.size_in_use),
                    info.size_allocated,
                    info.size_allocated,
                    info.blocks_in_use,
                    0
                );

                Ok(GetMemInfoRes::MallocInfo(info_str))
            }
            _ => Err(ControlError::UnknownMemInfoMode {
                mode: mode.to_string(),
            }
            .into()),
        }

        #[cfg(not(any(target_env = "gnu", target_os = "macos")))]
        // Just return zeroed stats for non-GNU and non-MacOS targets
        match mode {
            "stats" => Ok(GetMemInfoRes::Stats(GetMemInfoStats::default())),
            "mallocinfo" => Ok(GetMemInfoRes::MallocInfo(String::new())),
            _ => Err(ControlError::UnknownMemInfoMode {
                mode: mode.to_string(),
            }
            .into()),
        }
    }

    pub(super) async fn get_rpc_info(&self) -> Result<GetRpcInfoRes, JsonRpcError> {
        let active_commands = self
            .inflight
            .read()
            .await
            .values()
            .map(|req| ActiveCommand {
                method: req.method.clone(),
                duration: req.when.elapsed().as_micros() as u64,
            })
            .collect();

        Ok(GetRpcInfoRes {
            active_commands,
            logpath: self.log_path.clone(),
        })
    }

    // help
    // logging

    // stop
    pub(super) async fn stop(&self) -> Result<&str, JsonRpcError> {
        *self.kill_signal.write().await = true;

        Ok("Floresta stopping")
    }

    // uptime
    pub(super) fn uptime(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GetMemInfoStats {
    locked: MemInfoLocked,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MemInfoLocked {
    used: u64,
    free: u64,
    total: u64,
    locked: u64,
    chunks_used: u64,
    chunks_free: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetMemInfoRes {
    Stats(GetMemInfoStats),
    MallocInfo(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActiveCommand {
    method: String,
    duration: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetRpcInfoRes {
    active_commands: Vec<ActiveCommand>,
    logpath: PathBuf,
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
    use crate::json_rpc::res::jsonrpc_interface::JsonRpcError;
    use crate::json_rpc::test_fixture::test_rpc;

    /// `getmemoryinfo` only understands two modes; anything else must be rejected by name
    /// rather than silently defaulting to one of them.
    #[test]
    fn propagates_invalid_mem_info_mode() {
        let fixture = test_rpc();

        for mode in ["", "bogus", "STATS", "malloc"] {
            let err = fixture.rpc.get_memory_info(mode).unwrap_err();
            assert!(
                matches!(err, JsonRpcError::InvalidMemInfoMode),
                "mode {mode:?} should be rejected"
            );
        }
    }

    /// The documented modes are accepted, so the rejection above is discriminating rather
    /// than refusing everything.
    #[test]
    fn accepts_the_documented_mem_info_modes() {
        let fixture = test_rpc();

        assert!(fixture.rpc.get_memory_info("stats").is_ok());
        assert!(fixture.rpc.get_memory_info("mallocinfo").is_ok());
    }

    /// Uptime is derived from the recorded start time and must be monotonic, not a panic
    /// waiting on a clock that moved backwards.
    #[test]
    fn uptime_is_non_negative() {
        let fixture = test_rpc();

        // Just needs to not panic and to be readable twice.
        let first = fixture.rpc.uptime();
        let second = fixture.rpc.uptime();

        assert!(second >= first);
    }
}
