//! Networking: the transport, Team Create sessions, and the sync of scenes and text between them.
//!
//! Owns: the wire format ([`codec`]), the TCP transport on background threads ([`transport`]), identities
//! ([`identity`]) and Team Create's sessions ([`collab`]). See `docs/adr/0011-team-create.md`.
//!
//! Must not depend on: the render, scripting, project or platform crates. Only `std::net` reaches the network, so
//! it builds for every target the engine does. Nothing here blocks its caller: sockets live on threads of their own,
//! and the caller moves bytes through channels.

pub mod codec;
pub mod collab;
pub mod identity;
pub mod transport;

use metallic_core::{PropType, Registry};

/// A number that is the same for two class registries exactly when they declare the same classes, properties (and
/// their types) and enums, in the same order: peers must agree on it, because operations name classes and
/// properties by their index.
pub fn registry_fingerprint(registry: &Registry) -> u64 {
    let mut hash = Fnv::new();
    for class in registry.classes() {
        hash.str(class.name());
        for prop in class.props() {
            hash.str(prop.meta.name);
            match prop.meta.ty {
                PropType::Enum(name) | PropType::Ref(name) => hash.str(name),
                other => hash.str(&other.to_string()),
            }
            hash.byte(u8::from(prop.hidden));
        }
        hash.byte(0xff);
    }
    for desc in registry.enums() {
        hash.str(desc.name);
        for item in desc.items {
            hash.str(item);
        }
    }
    hash.0
}

/// FNV-1a, 64 bits: stable across builds and platforms, unlike the standard library's hasher.
pub(crate) struct Fnv(pub(crate) u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
    }

    fn str(&mut self, text: &str) {
        for &b in text.as_bytes() {
            self.byte(b);
        }
        self.byte(0);
    }
}
