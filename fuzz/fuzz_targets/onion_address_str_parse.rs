#![no_main]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "fuzz harness: a panic is how the fuzzer reports a finding"
)]
use std::str;

use floresta_wire::onion::OnionV3Addr;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(address_str) = str::from_utf8(data) else {
        return;
    };

    let _ = address_str.parse::<OnionV3Addr>();
});
