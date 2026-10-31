//! Writes `design/tokens/metallic-tokens.css` from `design/tokens/metallic.tokens.json`.
//! Run: `cargo run -p metallic_tokens --example export_css`.

use std::path::Path;

fn main() {
    let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).expect("tokens are valid");
    let target =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/tokens/metallic-tokens.css");
    std::fs::write(&target, metallic_tokens::to_css(&set)).expect("write the CSS file");
    println!("wrote design/tokens/metallic-tokens.css");
}
