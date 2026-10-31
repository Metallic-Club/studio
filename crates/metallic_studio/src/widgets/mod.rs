//! Studio's own widgets on top of egui: the cut, plates, buttons and keycaps (research doc
//! sections 3.3, 3.4 and 3.7). [`components`] draws all of them in every state for design review
//! (`--scene components`).

pub mod button;
pub mod components;
pub mod cut;
pub mod plate;

pub use button::{Button, Kind, ShowAs, keycap};
pub use plate::Plate;
