//! Metallic networking: transports, and (as later tasks land) wire encoding, replication, snapshots and prediction.
//!
//! Depends on `metallic_core` only, never on the runtime or render. See
//! `docs/superpowers/specs/2026-10-30-multiplayer-m2-design.md`.

#![warn(missing_docs)]

pub mod error;
pub mod transport;

pub use error::NetError;
