//! Colour editing for building (design spec `docs/design/building-tools.md`, section 11).
//!
//! - [`maths`]: sRGB bytes, HSV, hex and CSS names, OKLab/OKLCh, harmonies, typed expressions.
//! - [`palette`]: recent colours (per user) and saved palettes (`palettes/<name>.palette.ron`).
//! - [`popover`]: the [`ColourPopover`] egui widget. It returns [`ColourEvent`]s (preview, commit,
//!   cancel, eyedropper clicks) and never edits the scene itself.
//! - [`demo`]: a stand-alone screen (`--scene colour`) with a demo Part, until the editor's
//!   Properties panel (#10) and preview overlay (#213) exist to host the popover.

pub mod css_names;
pub mod demo;
pub mod maths;
pub mod palette;
pub mod popover;

pub use maths::{Harmony, Hsv, Oklch, Rgb8, parse_colour};
pub use palette::{Palette, PaletteStore, RecentColours};
pub use popover::{ColourEvent, ColourLibrary, ColourPopover};

#[cfg(test)]
mod tests;
