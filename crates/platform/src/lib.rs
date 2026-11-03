//! The platform layer: the only engine crate that knows which operating system it runs on.
//!
//! Owns: windows and the event loop, keyboard, mouse, gamepad and touch input, user config paths, and the seams
//! that private console crates implement. `#[cfg(target_os = ...)]` is allowed here and in app packaging only.
//!
//! Must not depend on: any other Metallic crate. See `docs/adr/0004-runtime-rendering-platform.md`.
