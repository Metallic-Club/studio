//! Atoms: small ids Luau caches in strings that name engine members, so a member lookup indexes an array.
//!
//! Luau asks for a string's atom the first time a C function reads it with `lua_tolstringatom` or
//! `lua_namecallatom` (and when it loads a chunk), then keeps it in the string. Strings are interned, so every
//! `"Position"` in every script shares one atom, and `part.Position` costs one array index per class instead of a
//! hash of the name.
//!
//! The table is process-wide because Luau's callback gets no backend state, and it only grows: each backend adds
//! the member names of its registry when it is created. A string that is not a member name gets atom -1, and its
//! lookups take the slow path by name (which only finds something if another registry added the name later).

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use metallic_core::Registry;

/// The member names known so far and their atoms.
#[derive(Default)]
struct Table {
    atoms: HashMap<Box<str>, i16>,
}

static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(|| RwLock::new(Table::default()));

/// Names every script-visible member of `registry` (properties and their aliases, methods, signals, callbacks) plus
/// `extra`,
/// so their strings get atoms. Past `i16::MAX` names, the rest go without (and take the slow path).
pub(crate) fn register(registry: &Registry, extra: &[&str]) {
    let Ok(mut table) = TABLE.write() else {
        return;
    };
    let mut add = |name: &str| {
        if table.atoms.contains_key(name) {
            return;
        }
        if let Ok(atom) = i16::try_from(table.atoms.len()) {
            table.atoms.insert(name.into(), atom);
        }
    };
    for class in registry.classes() {
        for prop in class.props() {
            add(prop.meta.name);
            for alias in prop.meta.aliases {
                add(alias);
            }
        }
        for method in class.methods() {
            add(method.desc.name);
        }
        for signal in class.signals() {
            add(signal.desc.name);
        }
        for callback in class.callbacks() {
            add(callback.desc.name);
        }
    }
    for name in extra {
        add(name);
    }
}

/// The atom of a member name, or -1. Luau calls this once per distinct string, through `ffi::useratom`.
pub(crate) fn atom_of(bytes: &[u8]) -> i16 {
    // Member names are short identifiers; anything else is not worth a lookup.
    if bytes.is_empty() || bytes.len() > 64 {
        return -1;
    }
    let Ok(name) = std::str::from_utf8(bytes) else {
        return -1;
    };
    TABLE
        .read()
        .ok()
        .and_then(|table| table.atoms.get(name).copied())
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ffi::MemberName` skips the UTF-8 check of a name with an atom, which holds because only names in the
    /// table (all `str`s) get one.
    #[test]
    fn only_utf8_member_names_get_atoms() {
        let registry = metallic_core::Registry::build(metallic_classes::input()).unwrap();
        register(&registry, &[]);
        assert!(atom_of(b"Position") >= 0);
        assert_eq!(atom_of(b"Posi\xfftion"), -1);
        assert_eq!(atom_of(b"\xff"), -1);
        assert_eq!(atom_of(b"Position\xff"), -1);
    }
}
