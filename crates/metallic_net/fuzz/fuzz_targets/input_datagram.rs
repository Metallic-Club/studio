//! Datagrams a client sends: inputs and unreliable remote calls.

#![no_main]

use libfuzzer_sys::fuzz_target;
use metallic_net::wire::{decode_input, decode_unreliable_remote};

fuzz_target!(|data: &[u8]| {
    let _ = decode_input(data);
    let _ = decode_unreliable_remote(data);
});
