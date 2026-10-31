//! Metallic design tokens as Rust constants. Generated from
//! `design/tokens/metallic.tokens.json`: do not edit. Regenerate with:
//! `cargo run -p metallic_tokens --example export_rust`.
//!
//! Colours are unmultiplied `[r, g, b, a]`, dimensions are px, durations are seconds. Theme and
//! reduced-motion overrides and font families are not included.

/// `primitive.g.950`
pub const G_950: [u8; 4] = [12, 11, 15, 255];

/// `primitive.g.900`
pub const G_900: [u8; 4] = [17, 16, 21, 255];

/// `primitive.g.850`
pub const G_850: [u8; 4] = [22, 21, 27, 255];

/// `primitive.g.800`
pub const G_800: [u8; 4] = [28, 27, 34, 255];

/// `primitive.g.750`
pub const G_750: [u8; 4] = [36, 34, 43, 255];

/// `primitive.g.700`
pub const G_700: [u8; 4] = [44, 42, 52, 255];

/// `primitive.g.650`
pub const G_650: [u8; 4] = [53, 50, 62, 255];

/// `primitive.g.600`
pub const G_600: [u8; 4] = [67, 64, 77, 255];

/// `primitive.g.500`
pub const G_500: [u8; 4] = [95, 91, 107, 255];

/// `primitive.g.400`
pub const G_400: [u8; 4] = [129, 125, 140, 255];

/// `primitive.g.300`
pub const G_300: [u8; 4] = [163, 160, 173, 255];

/// `primitive.g.200`
pub const G_200: [u8; 4] = [195, 192, 202, 255];

/// `primitive.g.100`
pub const G_100: [u8; 4] = [230, 228, 234, 255];

/// `primitive.g.50`
pub const G_50: [u8; 4] = [244, 243, 247, 255];

/// `primitive.v.50`
pub const V_50: [u8; 4] = [243, 240, 252, 255];

/// `primitive.v.100`
pub const V_100: [u8; 4] = [228, 222, 248, 255];

/// `primitive.v.200`
pub const V_200: [u8; 4] = [205, 194, 242, 255];

/// `primitive.v.300`
pub const V_300: [u8; 4] = [180, 166, 236, 255];

/// `primitive.v.400`
pub const V_400: [u8; 4] = [157, 139, 224, 255];

/// `primitive.v.500`
pub const V_500: [u8; 4] = [134, 115, 207, 255];

/// `primitive.v.600`
pub const V_600: [u8; 4] = [111, 92, 182, 255];

/// `primitive.v.700`
pub const V_700: [u8; 4] = [88, 73, 147, 255];

/// `primitive.v.800`
pub const V_800: [u8; 4] = [58, 51, 84, 255];

/// `primitive.v.900`
pub const V_900: [u8; 4] = [42, 38, 56, 255];

/// `primitive.v.950`
pub const V_950: [u8; 4] = [31, 28, 41, 255];

/// `role.bg-deep` (= `primitive.g.900`)
pub const BG_DEEP: [u8; 4] = [17, 16, 21, 255];

/// `role.bg-base` (= `primitive.g.850`)
pub const BG_BASE: [u8; 4] = [22, 21, 27, 255];

/// `role.bg-panel` (= `primitive.g.800`)
pub const BG_PANEL: [u8; 4] = [28, 27, 34, 255];

/// `role.bg-raised` (= `primitive.g.750`)
pub const BG_RAISED: [u8; 4] = [36, 34, 43, 255];

/// `role.bg-hover` (= `primitive.g.700`)
pub const BG_HOVER: [u8; 4] = [44, 42, 52, 255];

/// `role.bg-active` (= `primitive.g.650`)
pub const BG_ACTIVE: [u8; 4] = [53, 50, 62, 255];

/// `role.select` (= `primitive.v.800`)
pub const SELECT: [u8; 4] = [58, 51, 84, 255];

/// `role.select-weak` (= `primitive.v.900`)
pub const SELECT_WEAK: [u8; 4] = [42, 38, 56, 255];

/// `role.line`
pub const LINE: [u8; 4] = [43, 41, 51, 255];

/// `role.line-strong` (= `primitive.g.600`)
pub const LINE_STRONG: [u8; 4] = [67, 64, 77, 255];

/// `role.line-control`
pub const LINE_CONTROL: [u8; 4] = [110, 106, 121, 255];

/// `role.text-1`
pub const TEXT_1: [u8; 4] = [230, 228, 234, 255];

/// `role.text-2`
pub const TEXT_2: [u8; 4] = [174, 170, 182, 255];

/// `role.text-3`
pub const TEXT_3: [u8; 4] = [142, 138, 151, 255];

/// `role.text-off` (= `primitive.g.500`)
pub const TEXT_OFF: [u8; 4] = [95, 91, 107, 255];

/// `role.icon`
pub const ICON: [u8; 4] = [189, 186, 196, 255];

/// `role.accent` (= `primitive.v.400`)
pub const ACCENT: [u8; 4] = [157, 139, 224, 255];

/// `role.accent-hover` (= `primitive.v.300`)
pub const ACCENT_HOVER: [u8; 4] = [180, 166, 236, 255];

/// `role.accent-press` (= `primitive.v.500`)
pub const ACCENT_PRESS: [u8; 4] = [134, 115, 207, 255];

/// `role.accent-text` (= `primitive.v.300`)
pub const ACCENT_TEXT: [u8; 4] = [180, 166, 236, 255];

/// `role.on-accent` (= `primitive.g.900`)
pub const ON_ACCENT: [u8; 4] = [17, 16, 21, 255];

/// `role.focus` (= `primitive.v.300`)
pub const FOCUS: [u8; 4] = [180, 166, 236, 255];

/// `role.highlight`
pub const HIGHLIGHT: [u8; 4] = [255, 255, 255, 15];

/// `role.error`
pub const ERROR: [u8; 4] = [236, 122, 114, 255];

/// `role.error-bg`
pub const ERROR_BG: [u8; 4] = [58, 34, 38, 255];

/// `role.warning`
pub const WARNING: [u8; 4] = [224, 173, 85, 255];

/// `role.warning-bg`
pub const WARNING_BG: [u8; 4] = [53, 44, 29, 255];

/// `role.success`
pub const SUCCESS: [u8; 4] = [98, 191, 145, 255];

/// `role.success-bg`
pub const SUCCESS_BG: [u8; 4] = [29, 51, 41, 255];

/// `role.info`
pub const INFO: [u8; 4] = [127, 176, 230, 255];

/// `role.info-bg`
pub const INFO_BG: [u8; 4] = [28, 42, 58, 255];

/// `role.danger`
pub const DANGER: [u8; 4] = [143, 58, 53, 255];

/// `role.danger-hover`
pub const DANGER_HOVER: [u8; 4] = [163, 68, 62, 255];

/// `space.s1`
pub const S1: f32 = 2.0;

/// `space.s2`
pub const S2: f32 = 4.0;

/// `space.s3`
pub const S3: f32 = 6.0;

/// `space.s4`
pub const S4: f32 = 8.0;

/// `space.s5`
pub const S5: f32 = 12.0;

/// `space.s6`
pub const S6: f32 = 16.0;

/// `space.s7`
pub const S7: f32 = 20.0;

/// `space.s8`
pub const S8: f32 = 24.0;

/// `space.s9`
pub const S9: f32 = 32.0;

/// `space.s10`
pub const S10: f32 = 40.0;

/// `space.s11`
pub const S11: f32 = 48.0;

/// `radius.r-xs`
pub const R_XS: f32 = 3.0;

/// `radius.r-sm`
pub const R_SM: f32 = 4.0;

/// `radius.r-md`
pub const R_MD: f32 = 6.0;

/// `radius.r-lg`
pub const R_LG: f32 = 10.0;

/// `cut.cut-sm`
pub const CUT_SM: f32 = 6.0;

/// `cut.cut-md`
pub const CUT_MD: f32 = 10.0;

/// `cut.cut-lg`
pub const CUT_LG: f32 = 14.0;

/// `size.h-row`
pub const H_ROW: f32 = 24.0;

/// `size.h-control`
pub const H_CONTROL: f32 = 32.0;

/// `size.h-control-lg`
pub const H_CONTROL_LG: f32 = 36.0;

/// `shadow.shadow-pop`
pub const SHADOW_POP: crate::Shadow = crate::Shadow {
    offset: [0.0, 8.0],
    blur: 24.0,
    spread: 0.0,
    color: [0, 0, 0, 115],
};

/// `shadow.shadow-modal`
pub const SHADOW_MODAL: crate::Shadow = crate::Shadow {
    offset: [0.0, 16.0],
    blur: 48.0,
    spread: 0.0,
    color: [0, 0, 0, 140],
};

/// `motion.t-fast`
pub const T_FAST: f32 = 0.09;

/// `motion.t-base`
pub const T_BASE: f32 = 0.14;

/// `motion.t-slow`
pub const T_SLOW: f32 = 0.2;

/// `motion.ease-out`
pub const EASE_OUT: [f32; 4] = [0.215, 0.61, 0.355, 1.0];

/// `motion.ease-in-out`
pub const EASE_IN_OUT: [f32; 4] = [0.645, 0.045, 0.355, 1.0];

/// `type.t-display.size`
pub const T_DISPLAY_SIZE: f32 = 30.0;

/// `type.t-display.line`
pub const T_DISPLAY_LINE: f32 = 36.0;

/// `type.t-display.weight`
pub const T_DISPLAY_WEIGHT: f32 = 700.0;

/// `type.t-title.size`
pub const T_TITLE_SIZE: f32 = 24.0;

/// `type.t-title.line`
pub const T_TITLE_LINE: f32 = 30.0;

/// `type.t-title.weight`
pub const T_TITLE_WEIGHT: f32 = 680.0;

/// `type.t-heading.size`
pub const T_HEADING_SIZE: f32 = 15.0;

/// `type.t-heading.line`
pub const T_HEADING_LINE: f32 = 20.0;

/// `type.t-heading.weight`
pub const T_HEADING_WEIGHT: f32 = 620.0;

/// `type.t-body.size`
pub const T_BODY_SIZE: f32 = 13.0;

/// `type.t-body.line`
pub const T_BODY_LINE: f32 = 18.0;

/// `type.t-body.weight`
pub const T_BODY_WEIGHT: f32 = 400.0;

/// `type.t-body-lg.size`
pub const T_BODY_LG_SIZE: f32 = 14.0;

/// `type.t-body-lg.line`
pub const T_BODY_LG_LINE: f32 = 20.0;

/// `type.t-body-lg.weight`
pub const T_BODY_LG_WEIGHT: f32 = 400.0;

/// `type.t-strong.size`
pub const T_STRONG_SIZE: f32 = 13.0;

/// `type.t-strong.line`
pub const T_STRONG_LINE: f32 = 18.0;

/// `type.t-strong.weight`
pub const T_STRONG_WEIGHT: f32 = 560.0;

/// `type.t-small.size`
pub const T_SMALL_SIZE: f32 = 12.0;

/// `type.t-small.line`
pub const T_SMALL_LINE: f32 = 16.0;

/// `type.t-small.weight`
pub const T_SMALL_WEIGHT: f32 = 400.0;

/// `type.t-caption.size`
pub const T_CAPTION_SIZE: f32 = 11.0;

/// `type.t-caption.line`
pub const T_CAPTION_LINE: f32 = 14.0;

/// `type.t-caption.weight`
pub const T_CAPTION_WEIGHT: f32 = 520.0;

/// `type.t-mono.size`
pub const T_MONO_SIZE: f32 = 12.0;

/// `type.t-mono.line`
pub const T_MONO_LINE: f32 = 18.0;

/// `type.t-mono.weight`
pub const T_MONO_WEIGHT: f32 = 400.0;
