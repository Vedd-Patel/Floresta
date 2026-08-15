// SPDX-License-Identifier: MIT OR Apache-2.0

#![no_main]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "fuzz harness: a panic is how the fuzzer reports a finding"
)]
use bitcoin::consensus::deserialize;
use floresta_wire::block_proof::UtreexoProof;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = deserialize::<UtreexoProof>(data);
});
