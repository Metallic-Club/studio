//! The one place the editor panels read colours, font sizes and metrics from.
//!
//! Every editor module (viewport, gizmos, Explorer, Properties, Output, script tabs, toolbar
//! icons) calls [`ui_tokens`] instead of naming colours or sizes itself. Today the values come from
//! [`crate::theme`] plus a few editor-only additions; when the theme moves to `metallic_tokens`
//! (S2), only this file changes.
use eframe::egui::{Color32, FontId};

use crate::theme;

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

const TOKENS: UiTokens = UiTokens {
    deep: theme::DEEP,
    panel: theme::PANEL,
    hover: theme::HOVER,
    accent: theme::ACCENT,
    text: theme::TEXT,
    text_dim: theme::TEXT_DIM,
    error: theme::ERROR,
    warning: Color32::from_rgb(0xe0, 0xc0, 0x7a),
    error_bg: Color32::from_rgb(0x3a, 0x22, 0x26),
    success: Color32::from_rgb(0x8f, 0xc9, 0x9a),
    axis: [
        Color32::from_rgb(0xe0, 0x6c, 0x6c),
        Color32::from_rgb(0x84, 0xc9, 0x6a),
        Color32::from_rgb(0x6c, 0x9c, 0xe8),
    ],
    icon: Color32::from_rgb(0xc4, 0xc6, 0xcc),
    icon_hole: theme::BASE,
    clients: [
        Color32::from_rgb(0x7f, 0xb0, 0xd8),
        Color32::from_rgb(0xd8, 0xb0, 0x6f),
        Color32::from_rgb(0x8f, 0xc9, 0x9a),
        Color32::from_rgb(0xd8, 0x8f, 0xb8),
    ],
    other_tag: Color32::from_rgb(0xb8, 0xb4, 0xc2),
    syntax_keyword: Color32::from_rgb(0xb3, 0x9d, 0xe6),
    syntax_string: Color32::from_rgb(0x9f, 0xc9, 0x8f),
    syntax_number: Color32::from_rgb(0xe0, 0xb4, 0x7a),
    syntax_comment: Color32::from_rgb(0x8e, 0x8a, 0x97),
    body_size: 13.0,
    small_size: 11.0,
    mono_size: 12.0,
    title_size: 14.0,
    row_height: 24.0,
    indent: 18.0,
    label_width: 104.0,
    icon_size: 16.0,
};

/// The editor's tokens.
pub fn ui_tokens() -> &'static UiTokens {
    &TOKENS
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
