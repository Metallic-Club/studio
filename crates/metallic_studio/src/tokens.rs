//! The one place the editor panels read colours, font sizes and metrics from.
//!
//! Every editor module (viewport, gizmos, Explorer, Properties, Output, script tabs, toolbar
//! icons) calls [`ui_tokens`] instead of naming colours or sizes itself. Today the values come from
//! [`crate::theme`] (the active theme's palette) plus a few editor-only additions.
use eframe::egui::{Color32, FontId};

use std::sync::OnceLock;

use crate::theme::{self, ThemeId};

/// Colours, font sizes and metrics used by the editor panels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiTokens {
    /// Deepest background (viewport without GPU, text fields).
    pub deep: Color32,
    /// Raised surfaces (labels over the viewport).
    pub panel: Color32,
    /// Hovered row.
    pub hover: Color32,
    /// Accent: selection, outlines, Server tag.
    pub accent: Color32,
    /// Body text.
    pub text: Color32,
    /// Secondary text.
    pub text_dim: Color32,
    /// Error text.
    pub error: Color32,
    /// Warning text.
    pub warning: Color32,
    /// Background of an error row in Output.
    pub error_bg: Color32,
    /// Play state (the only green).
    pub success: Color32,
    /// X, Y, Z axis colours (always shown with their letters).
    pub axis: [Color32; 3],
    /// Icon colour (silver).
    pub icon: Color32,
    /// Cut-outs inside filled icons.
    pub icon_hole: Color32,
    /// Output tag chips for clients 1, 2, 3, 4 (then repeating).
    pub clients: [Color32; 4],
    /// Output tag chip for other sources.
    pub other_tag: Color32,
    /// Luau keywords.
    pub syntax_keyword: Color32,
    /// Luau strings.
    pub syntax_string: Color32,
    /// Luau numbers.
    pub syntax_number: Color32,
    /// Luau comments.
    pub syntax_comment: Color32,
    /// Row and label text size.
    pub body_size: f32,
    /// Small text (meta columns, status bar).
    pub small_size: f32,
    /// Code and numbers.
    pub mono_size: f32,
    /// Panel and header titles.
    pub title_size: f32,
    /// Explorer and Output row height.
    pub row_height: f32,
    /// Explorer indent per level.
    pub indent: f32,
    /// Properties label column width.
    pub label_width: f32,
    /// Icon box size.
    pub icon_size: f32,
}

impl UiTokens {
    /// Proportional font at body size.
    pub fn body(&self) -> FontId {
        FontId::proportional(self.body_size)
    }

    /// Proportional font at small size.
    pub fn small(&self) -> FontId {
        FontId::proportional(self.small_size)
    }

    /// Monospace font.
    pub fn mono(&self) -> FontId {
        FontId::monospace(self.mono_size)
    }
}

/// The editor's tokens for `id`. Roles the theme names come from its palette; the editor-only
/// colours (axes, tags, Luau syntax) are tuned per theme for contrast on `bg-deep`.
fn build(id: ThemeId) -> UiTokens {
    let p = theme::palette_of(id);
    let light = !id.is_dark();
    let pick = |dark: [u8; 3], light_value: [u8; 3]| {
        let [r, g, b] = if light { light_value } else { dark };
        Color32::from_rgb(r, g, b)
    };
    UiTokens {
        deep: p.bg_deep,
        panel: p.bg_panel,
        hover: p.bg_hover,
        accent: p.accent,
        text: p.text_1,
        text_dim: p.text_2,
        error: p.error,
        warning: if light {
            p.warning
        } else {
            Color32::from_rgb(0xe0, 0xc0, 0x7a)
        },
        error_bg: p.error_bg,
        success: if light {
            p.success
        } else {
            Color32::from_rgb(0x8f, 0xc9, 0x9a)
        },
        axis: [
            pick([0xe0, 0x6c, 0x6c], [0xb3, 0x26, 0x1e]),
            pick([0x84, 0xc9, 0x6a], [0x2c, 0x7a, 0x1e]),
            pick([0x6c, 0x9c, 0xe8], [0x1f, 0x5c, 0xb0]),
        ],
        icon: if light {
            p.icon
        } else {
            Color32::from_rgb(0xc4, 0xc6, 0xcc)
        },
        icon_hole: p.bg_base,
        clients: [
            pick([0x7f, 0xb0, 0xd8], [0x1f, 0x5c, 0x9e]),
            pick([0xd8, 0xb0, 0x6f], [0x7a, 0x52, 0x00]),
            pick([0x8f, 0xc9, 0x9a], [0x1d, 0x70, 0x48]),
            pick([0xd8, 0x8f, 0xb8], [0x9a, 0x2f, 0x6e]),
        ],
        other_tag: pick([0xb8, 0xb4, 0xc2], [0x57, 0x53, 0x5f]),
        syntax_keyword: pick([0xb3, 0x9d, 0xe6], [0x58, 0x49, 0x93]),
        syntax_string: pick([0x9f, 0xc9, 0x8f], [0x1d, 0x70, 0x48]),
        syntax_number: pick([0xe0, 0xb4, 0x7a], [0x8a, 0x4b, 0x00]),
        syntax_comment: p.text_3,
        body_size: 13.0,
        small_size: 11.0,
        mono_size: 12.0,
        title_size: 14.0,
        row_height: 24.0,
        indent: 18.0,
        label_width: 104.0,
        icon_size: 16.0,
    }
}

/// The editor's tokens in the active theme (see [`theme::apply`]).
pub fn ui_tokens() -> &'static UiTokens {
    static TOKENS: OnceLock<[UiTokens; 3]> = OnceLock::new();
    &TOKENS.get_or_init(|| ThemeId::ALL.map(build))[theme::active() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::contrast_ratio;

    #[test]
    fn editor_text_colours_are_readable() {
        let t = ui_tokens();
        for c in [
            t.warning,
            t.success,
            t.error,
            t.syntax_comment,
            t.syntax_keyword,
        ] {
            assert!(contrast_ratio(c, t.deep) >= 4.5, "{c:?}");
        }
        assert!(contrast_ratio(t.error, t.error_bg) >= 4.5);
        assert!(contrast_ratio(t.text, t.error_bg) >= 4.5);
    }
}
