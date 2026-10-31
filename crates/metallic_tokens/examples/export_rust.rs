//! Writes `crates/metallic_tokens/src/consts.rs` from `design/tokens/metallic.tokens.json`.
//! Run: `cargo run -p metallic_tokens --example export_rust`.

use std::path::Path;

fn main() {
    let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).expect("tokens are valid");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/consts.rs");
    std::fs::write(&target, metallic_tokens::to_rust(&set)).expect("write the Rust file");
    println!("wrote crates/metallic_tokens/src/consts.rs");
}
