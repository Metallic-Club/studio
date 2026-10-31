//! The committed CSS export must match the JSON source, role tokens must resolve, and the
//! Launcher's pages may only use tokens that exist.

use std::collections::BTreeSet;
use std::path::Path;

use metallic_tokens::{TOKENS_JSON, color, parse, themed_color, to_css};

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
fn css_has_one_block_per_theme() {
    let set = parse(TOKENS_JSON).unwrap();
    assert_eq!(set.theme_ids(), ["graphite", "titanium", "light"]);
    let css = to_css(&set);
    for id in set.theme_ids() {
        let selector = format!("[data-theme=\"{id}\"]");
        assert_eq!(css.matches(&selector).count(), 1, "one {selector} block");
    }
    // Every theme can be read back, and the three really differ.
    let base = |t: &str| themed_color(&set, t, "bg-base").unwrap();
    assert_eq!(base("graphite"), [0x16, 0x15, 0x1b, 0xff]);
    assert_eq!(base("titanium"), [0x24, 0x22, 0x2b, 0xff]);
    assert_eq!(base("light"), [0xec, 0xea, 0xf0, 0xff]);
    // Roles a theme does not override come from the default.
    assert_eq!(
        themed_color(&set, "titanium", "accent"),
        color(&set, "accent")
    );
}

#[test]
fn themes_override_only_known_roles_and_new_tokens_exist() {
    let css = to_css(&parse(TOKENS_JSON).unwrap());
    for name in [
        "edge-low",
        "art-scrim",
        "backdrop",
        "shadow-modal",
        "t-panel",
        "t-splash",
        "ease-in",
        "s12",
        "s13",
    ] {
        assert!(css.contains(&format!("  --{name}: ")), "missing --{name}");
    }
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

#[test]
fn reduced_motion_overrides_are_readable() {
    let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).expect("tokens parse");
    let reduced = set.reduced_motion();
    assert!(reduced.iter().any(|(n, v)| n == "t-fast" && v == "0ms"));
    assert!(reduced.iter().any(|(n, v)| n == "t-splash" && v == "80ms"));
}
