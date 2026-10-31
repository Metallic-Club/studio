//! The committed Rust constants must match the JSON source.

use metallic_tokens::{TOKENS_JSON, color, consts, parse, to_rust};

#[test]
fn committed_consts_are_current() {
    // A Windows checkout may turn the line ends into CRLF: compare without them.
    let rust = to_rust(&parse(TOKENS_JSON).unwrap()).replace("\r\n", "\n");
    let committed = include_str!("../src/consts.rs").replace("\r\n", "\n");
    assert_eq!(
        rust, committed,
        "regenerate with: cargo run -p metallic_tokens --example export_rust"
    );
}

#[test]
fn consts_follow_references() {
    let set = parse(TOKENS_JSON).unwrap();
    assert_eq!(Some(consts::BG_BASE), color(&set, "bg-base"));
    assert_eq!(Some(consts::HIGHLIGHT), color(&set, "highlight"));
    assert_eq!(consts::BG_BASE, consts::G_850);
    assert_eq!(consts::R_SM, 4.0);
    assert_eq!(consts::T_FAST, 0.09);
    assert_eq!(consts::SHADOW_POP.offset, [0.0, 8.0]);
    assert_eq!(consts::SHADOW_POP.color, [0, 0, 0, 115]);
}
