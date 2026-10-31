//! WCAG 2 contrast of the colour roles, in every theme: text must reach 4.5:1 on the surface it is
//! drawn on, icons and control borders 3:1. Disabled text (`text-off`) is exempt by WCAG.

use metallic_tokens::{TOKENS_JSON, TokenSet, parse, themed_color};

const TEXT: f64 = 4.5;
const CONTROL: f64 = 3.0;

/// The surfaces that carry text, from deepest to most pressed.
const SURFACES: [&str; 6] = [
    "bg-deep",
    "bg-base",
    "bg-panel",
    "bg-raised",
    "bg-hover",
    "bg-active",
];
/// The surfaces hints, links and status colours are drawn on (not the hover and pressed fills).
const RESTING: [&str; 4] = ["bg-deep", "bg-base", "bg-panel", "bg-raised"];

/// One foreground role on one background role, and the ratio it must reach.
struct Pair {
    fg: &'static str,
    bg: &'static str,
    min: f64,
}

fn pair(fg: &'static str, bg: &'static str, min: f64) -> Pair {
    Pair { fg, bg, min }
}

/// Every pair the interface draws.
fn real_pairs() -> Vec<Pair> {
    let mut out = Vec::new();
    for bg in SURFACES {
        out.push(pair("text-1", bg, TEXT));
        out.push(pair("text-2", bg, TEXT));
        out.push(pair("icon", bg, CONTROL));
    }
    for bg in RESTING {
        for fg in [
            "text-3",
            "accent-text",
            "error",
            "warning",
            "success",
            "info",
        ] {
            out.push(pair(fg, bg, TEXT));
        }
        for fg in ["line-control", "focus", "accent"] {
            out.push(pair(fg, bg, CONTROL));
        }
    }
    for status in ["error", "warning", "success", "info"] {
        let bg = match status {
            "error" => "error-bg",
            "warning" => "warning-bg",
            "success" => "success-bg",
            _ => "info-bg",
        };
        out.push(pair(status, bg, TEXT));
    }
    for bg in ["accent", "accent-hover", "accent-press"] {
        out.push(pair("on-accent", bg, TEXT));
    }
    for bg in ["danger", "danger-hover"] {
        out.push(pair("on-danger", bg, TEXT));
    }
    for fg in ["text-1", "text-2"] {
        out.push(pair(fg, "select", TEXT));
    }
    out.push(pair("text-1", "select-weak", TEXT));
    out
}

fn channel(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

/// The contrast ratio of two opaque colours, 1 to 21.
fn ratio(a: [u8; 4], b: [u8; 4]) -> f64 {
    let lum = |c: [u8; 4]| luminance([c[0].into(), c[1].into(), c[2].into()]);
    let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
    (hi + 0.05) / (lo + 0.05)
}

/// `fg` laid over `bg` (a foreground with alpha is blended first).
fn over(fg: [u8; 4], bg: [u8; 4]) -> [u8; 4] {
    let a = f64::from(fg[3]) / 255.0;
    let mix = |i: usize| (f64::from(fg[i]) * a + f64::from(bg[i]) * (1.0 - a)).round() as u8;
    [mix(0), mix(1), mix(2), 255]
}

/// The pairs that fall short in `theme`, each as a readable line.
fn failures(set: &TokenSet, theme: &str, pairs: &[Pair]) -> Vec<String> {
    let mut out = Vec::new();
    for p in pairs {
        let (Some(fg), Some(bg)) = (
            themed_color(set, theme, p.fg),
            themed_color(set, theme, p.bg),
        ) else {
            out.push(format!(
                "{theme}: {} or {} is not a colour role",
                p.fg, p.bg
            ));
            continue;
        };
        let got = ratio(over(fg, bg), bg);
        if got < p.min {
            out.push(format!(
                "{theme}: {} on {} is {got:.2}, needs {}",
                p.fg, p.bg, p.min
            ));
        }
    }
    out
}

#[test]
fn ratio_matches_known_values() {
    let white = [255, 255, 255, 255];
    let black = [0, 0, 0, 255];
    assert!((ratio(white, black) - 21.0).abs() < 1e-9);
    assert!((ratio(white, white) - 1.0).abs() < 1e-9);
    // #777777 on white is the classic 4.48 that just misses AA.
    assert!((ratio([0x77, 0x77, 0x77, 255], white) - 4.48).abs() < 0.01);
}

#[test]
fn a_planted_bad_pair_is_caught() {
    let set = parse(TOKENS_JSON).unwrap();
    // text-off on bg-panel is a disabled colour: about 2.4, far below 4.5.
    let planted = [
        pair("text-off", "bg-panel", TEXT),
        pair("text-1", "bg-panel", TEXT),
    ];
    let found = failures(&set, "graphite", &planted);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("text-off on bg-panel"));
    // A name that is not a role is reported too, not skipped.
    let unknown = failures(&set, "graphite", &[pair("text-9", "bg-panel", TEXT)]);
    assert_eq!(unknown.len(), 1);
}

#[test]
fn every_theme_meets_contrast() {
    let set = parse(TOKENS_JSON).unwrap();
    assert_eq!(set.theme_ids().len(), 3);
    let pairs = real_pairs();
    let mut all = Vec::new();
    for theme in set.theme_ids() {
        all.extend(failures(&set, theme, &pairs));
    }
    assert!(all.is_empty(), "contrast failures:\n{}", all.join("\n"));
}
