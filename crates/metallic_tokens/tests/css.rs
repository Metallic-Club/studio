//! The committed CSS export must match the JSON source, role tokens must resolve, and the
//! Launcher's pages may only use tokens that exist.

use std::collections::BTreeSet;
use std::path::Path;

use metallic_tokens::{TOKENS_JSON, color, parse, to_css};

#[test]
fn committed_css_is_current() {
    // A Windows checkout may turn the line ends into CRLF: compare without them.
    let css = to_css(&parse(TOKENS_JSON).unwrap()).replace("\r\n", "\n");
    let committed =
        include_str!("../../../design/tokens/metallic-tokens.css").replace("\r\n", "\n");
    assert_eq!(
        css, committed,
        "regenerate with: cargo run -p metallic_tokens --example export_css"
    );
}

#[test]
fn every_role_token_resolves() {
    let set = parse(TOKENS_JSON).unwrap();
    let mut roles = 0;
    for token in set.tokens().iter().filter(|t| t.path()[0] == "role") {
        roles += 1;
        if let Some(target) = token.reference() {
            let found = set.tokens().iter().find(|t| t.path().join(".") == target);
            let found =
                found.unwrap_or_else(|| panic!("{} points at missing {target}", token.name()));
            assert_eq!(
                found.path()[0],
                "primitive",
                "{} must alias a primitive",
                token.name()
            );
        }
    }
    assert!(roles > 20, "role tokens are missing");
    // Native code can read a role as a colour, through its alias.
    assert_eq!(color(&set, "bg-base"), Some([0x16, 0x15, 0x1b, 0xff]));
    assert_eq!(color(&set, "highlight"), Some([255, 255, 255, 15]));
    assert_eq!(color(&set, "font"), None);
    assert_eq!(color(&set, "nope"), None);
}

#[test]
fn launcher_pages_use_only_defined_tokens() {
    let css = to_css(&parse(TOKENS_JSON).unwrap());
    let defined: BTreeSet<&str> = css
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--")?.split(':').next())
        .collect();
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../metallic_launcher/assets/pages");
    let mut used = 0;
    for entry in std::fs::read_dir(pages).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "css") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for part in text.split("var(--").skip(1) {
            let name = part.split([')', ',']).next().unwrap().trim();
            used += 1;
            assert!(
                defined.contains(name),
                "{}: unknown token --{name}",
                path.display()
            );
        }
        // The page style sheet takes values from the tokens only: no token declaration and no
        // literal colour (what is left once the `var(--x)` uses are removed).
        let bare = text.replace("var(--", "var(");
        assert!(
            !bare.contains("--"),
            "{}: defines its own token",
            path.display()
        );
        assert!(!bare.contains("rgb"), "{}: literal colour", path.display());
        let hex = bare
            .split('#')
            .skip(1)
            .any(|r| r.chars().take_while(char::is_ascii_hexdigit).count() >= 3);
        assert!(!hex, "{}: literal colour", path.display());
    }
    assert!(used > 20, "no token use found");
}
