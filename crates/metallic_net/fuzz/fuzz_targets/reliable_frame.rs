//! Reliable frames from either side, and the join snapshot body inside a server frame.

#![no_main]

use libfuzzer_sys::fuzz_target;
use metallic_net::wire::{Decoded, ServerMsg, decode_client, decode_server, decompress_join};

fuzz_target!(|data: &[u8]| {
    let _ = decode_client(data);
    if let Ok(Decoded::Msg {
        msg: ServerMsg::JoinSnapshot { zstd, .. },
        ..
    }) = decode_server(data)
    {
        let _ = decompress_join(&zstd);
    }
});
