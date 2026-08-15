// SPDX-License-Identifier: MIT OR Apache-2.0

#![no_main]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "fuzz harness: a panic is how the fuzzer reports a finding"
)]
use bitcoin::p2p::message::NetworkMessage;
use bitcoin::p2p::message_blockdata::Inventory;
use floresta_wire::network_message_ext::NetworkMessageExt;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(message) = NetworkMessage::deserialize_v2(data) else {
        return;
    };

    // `getuproof` is an outbound-only extension and has no corresponding decode path.
    if matches!(
        &message,
        NetworkMessage::Unknown { command, .. } if command.as_ref() == "getuproof"
    ) {
        return;
    }

    // TODO: rust-bitcoin 0.32.8 does not consume the hash following an `Error`
    // inventory type when decoding. Fixed upstream in rust-bitcoin@72e97c6;
    // remove this workaround after upgrading.
    if matches!(
        &message,
        NetworkMessage::Inv(inventory)
            | NetworkMessage::GetData(inventory)
            | NetworkMessage::NotFound(inventory)
            if inventory.iter().any(|item| matches!(item, Inventory::Error))
    ) {
        return;
    }

    let encoded = message.serialize_v2();
    let decoded =
        NetworkMessage::deserialize_v2(&encoded).expect("serialized v2 message must decode");
    assert_eq!(decoded, message, "v2 message roundtrip mismatch");
});
