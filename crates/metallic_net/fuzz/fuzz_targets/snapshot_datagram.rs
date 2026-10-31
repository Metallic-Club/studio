//! Snapshot datagrams (server to client).

#![no_main]

use libfuzzer_sys::fuzz_target;
use metallic_net::wire::{datagram_kind, decode_snapshot};

fuzz_target!(|data: &[u8]| {
    let _ = datagram_kind(data);
    let _ = decode_snapshot(data);
});
