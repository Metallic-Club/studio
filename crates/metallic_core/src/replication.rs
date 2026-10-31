//! Core schema and visibility rules for a future server-to-client replicator.
//!
//! These functions do not authenticate a connection or send data. The server must supply a
//! viewer Player from its trusted, same-world connection roster. No viewer means public-only
//! observation; it never grants access to a player's private containers.
//! This is the play-client policy, not the separately authorized edit-session snapshot policy.

use crate::classes::{self, ClassDef, PropDef, Repl};
use crate::{InstanceId, Value, World};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Fingerprint of class order, inheritance, effective property order/types/defaults, enums and
/// replication/hidden metadata. Version 2 encodes boxed CFrames by their 12 row-major
/// components, never their allocation address. All lengths and numeric bits use little endian.
///
/// Protocol peers must compare this hash before interpreting class-relative property indices.
/// It is a compatibility fingerprint, not an authentication credential or a world-state hash.
pub fn registry_hash() -> [u8; 32] {
    static HASH: LazyLock<[u8; 32]> = LazyLock::new(|| hash_schema(classes::registry_classes()));
    *HASH
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn hash_value(hash: &mut Sha256, value: &Value) {
    field(hash, value.value_type().name().as_bytes());
    let mut number = |v: f64| hash.update(v.to_bits().to_le_bytes());
    match value {
        Value::Nil => {}
        Value::Bool(v) => hash.update([u8::from(*v)]),
        Value::Number(v) => number(*v),
        Value::String(v) => field(hash, v.as_bytes()),
        Value::Vector3(v) => {
            number(v.x);
            number(v.y);
            number(v.z);
        }
        Value::Color3(v) => {
            number(v.r);
            number(v.g);
            number(v.b);
        }
        Value::UDim2(v) => {
            number(v.x_scale);
            number(v.x_offset);
            number(v.y_scale);
            number(v.y_offset);
        }
        Value::CFrame(frame) => {
            for component in frame.components() {
                number(component);
            }
        }
        Value::Ref(v) => {
            hash.update([u8::from(v.is_some())]);
            if let Some(id) = v {
                hash.update(id.to_bits().to_le_bytes());
            }
        }
    }
}

fn hash_schema(classes: &[ClassDef]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"metallic-registry-v2\0");
    hash.update((classes.len() as u64).to_le_bytes());
    for class in classes {
        field(&mut hash, class.name.as_bytes());
        hash.update([u8::from(class.superclass.is_some())]);
        if let Some(parent) = class.superclass {
            field(&mut hash, parent.as_bytes());
        }
        let mut chain = vec![class];
        let mut parent = class.superclass;
        while let Some(name) = parent {
            let next = classes
                .iter()
                .find(|c| c.name == name)
                .expect("registered superclass");
            assert!(chain.len() < classes.len(), "registry inheritance cycle");
            chain.push(next);
            parent = next.superclass;
        }
        let properties: Vec<&PropDef> = chain.iter().rev().flat_map(|c| c.props).collect();
        assert!(
            properties.len() <= usize::from(u16::MAX) + 1,
            "property index overflow"
        );
        hash.update((properties.len() as u64).to_le_bytes());
        for property in properties {
            field(&mut hash, property.name.as_bytes());
            field(&mut hash, property.ty.name().as_bytes());
            hash.update([match property.repl {
                Repl::Replicated => 0,
                Repl::ServerOnly => 1,
            }]);
            hash.update([u8::from(property.hidden)]);
            hash.update([u8::from(property.allowed.is_some())]);
            if let Some(values) = property.allowed {
                hash.update((values.len() as u64).to_le_bytes());
                for value in values {
                    field(&mut hash, value.as_bytes());
                }
            }
            let value = if property.name == "Name" {
                Value::String(class.name.into())
            } else {
                (property.default)()
            };
            hash_value(&mut hash, &value);
        }
    }
    hash.finalize().into()
}

fn owner_of(world: &World, container: InstanceId) -> Option<InstanceId> {
    let mut current = world.parent(container);
    while let Some(id) = current {
        if world.class_name(id)? == "Player" {
            return Some(id);
        }
        current = world.parent(id);
    }
    None
}

/// Whether an instance belongs in the viewer's replicated tree.
///
/// ServerStorage/ServerScriptService subtrees, orphan roots and another player's private
/// containers are excluded. Privacy is class-based, so renaming a container cannot expose it.
/// The query is uncached: a reparent is reflected immediately. Invalid viewers fail closed.
pub fn instance_visible(world: &World, id: InstanceId, viewer: Option<InstanceId>) -> bool {
    if viewer.is_some_and(|player| {
        world.class_name(player) != Some("Player")
            || !world.root("Players").is_some_and(|players| {
                world.is_builtin(players) && world.parent(player) == Some(players)
            })
    }) {
        return false;
    }
    let mut current = Some(id);
    while let Some(node) = current {
        let Some(class) = world.class_name(node) else {
            return false;
        };
        if classes::is_subclass(class, "ServerStorage")
            || classes::is_subclass(class, "ServerScriptService")
        {
            return false;
        }
        if (classes::is_subclass(class, "PlayerGui")
            || classes::is_subclass(class, "PlayerScripts"))
            && (viewer.is_none() || owner_of(world, node) != viewer)
        {
            return false;
        }
        current = world.parent(node);
        if current.is_none() {
            return world.is_builtin(node)
                && matches!(
                    class,
                    "Workspace"
                        | "ReplicatedStorage"
                        | "Players"
                        | "Lighting"
                        | "StarterGui"
                        | "StarterPlayer"
                );
        }
    }
    false
}

fn property_allowed(class: &str, property: &PropDef) -> bool {
    property.repl == Repl::Replicated
        && !(property.name == "Source"
            && classes::is_subclass(class, "Script")
            && !classes::is_subclass(class, "LocalScript"))
}

/// Whether a property may be sent. A server Script's Source is private even under Workspace;
/// LocalScript and ModuleScript source remains eligible in visible containers.
pub fn property_visible(
    world: &World,
    id: InstanceId,
    property: &str,
    viewer: Option<InstanceId>,
) -> bool {
    if !instance_visible(world, id, viewer) {
        return false;
    }
    let Some(class) = world.class_name(id) else {
        return false;
    };
    classes::find_property(class, property).is_some_and(|def| property_allowed(class, def))
}

/// Copies one outgoing property after applying visibility. Properties excluded by replication/instance visibility return None;
/// references to invisible/destroyed targets become Ref(None). This does not mutate the world.
pub fn property_value(
    world: &World,
    id: InstanceId,
    property: &str,
    viewer: Option<InstanceId>,
) -> Option<Value> {
    if !property_visible(world, id, property, viewer) {
        return None;
    }
    let value = world.get(id, property).ok()?;
    Some(match value {
        Value::Ref(Some(target)) if !instance_visible(world, target, viewer) => Value::Ref(None),
        value => value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;

    fn base(prop: PropDef) -> ClassDef {
        ClassDef {
            name: "Fixture",
            superclass: None,
            props: Box::leak(vec![prop].into_boxed_slice()),
        }
    }

    fn prop() -> PropDef {
        PropDef {
            name: "Flag",
            ty: ValueType::Bool,
            default: || Value::Bool(false),
            allowed: None,
            repl: Repl::Replicated,
            hidden: false,
        }
    }

    #[test]
    fn fingerprint_covers_property_metadata_defaults_order_and_class_identity() {
        let original = hash_schema(&[base(prop())]);
        assert_eq!(original, hash_schema(&[base(prop())]));
        // Independent SHA-256 fixture from the documented byte encoding.
        assert_eq!(
            original,
            [
                0x6c, 0xd1, 0xb0, 0x03, 0xb2, 0x74, 0x25, 0x55, 0x78, 0x15, 0xbe, 0x76, 0xec, 0xf1,
                0x15, 0x41, 0x7f, 0x19, 0xdf, 0xe2, 0xe8, 0xfe, 0xd1, 0x12, 0xbe, 0x80, 0x64, 0x95,
                0x5d, 0x39, 0xc3, 0xf9
            ]
        );
        let mut changed = prop();
        changed.repl = Repl::ServerOnly;
        assert_ne!(original, hash_schema(&[base(changed)]));
        let mut changed = prop();
        changed.hidden = true;
        assert_ne!(original, hash_schema(&[base(changed)]));
        let mut changed = prop();
        changed.default = || Value::Bool(true);
        assert_ne!(original, hash_schema(&[base(changed)]));
        let mut changed = prop();
        changed.ty = ValueType::String;
        assert_ne!(original, hash_schema(&[base(changed)]));
        let mut changed = prop();
        changed.allowed = Some(&["A", "B"]);
        assert_ne!(original, hash_schema(&[base(changed)]));
        let mut changed = prop();
        changed.name = "Other";
        assert_ne!(original, hash_schema(&[base(changed)]));
        assert_ne!(
            original,
            hash_schema(&[
                base(prop()),
                ClassDef {
                    name: "Added",
                    superclass: None,
                    props: &[]
                }
            ])
        );
        let mut changed = base(prop());
        changed.name = "Changed";
        assert_ne!(original, hash_schema(&[changed]));
    }

    #[test]
    fn boxed_cframe_hash_covers_all_components_and_numeric_bits() {
        use crate::{CFrame, Vec3};
        let digest = |frame: CFrame| -> [u8; 32] {
            let mut hash = Sha256::new();
            hash_value(&mut hash, &Value::CFrame(Box::new(frame)));
            hash.finalize().into()
        };
        let frame = CFrame::from_parts(
            Vec3::new(1.25, -2.5, 3.75),
            [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // Independent Python SHA-256 over type tag plus 12 row-major LE doubles.
        assert_eq!(
            digest(frame),
            [
                0xde, 0x81, 0x8e, 0x00, 0xee, 0x3e, 0x96, 0xe8, 0x1f, 0x1b, 0x70, 0xc4, 0x27, 0x96,
                0xd2, 0x4a, 0x76, 0xb9, 0xcb, 0x0b, 0xcb, 0x7f, 0x0f, 0xf0, 0x7e, 0x42, 0x38, 0xfe,
                0xb8, 0x83, 0xac, 0x79
            ]
        );
        for index in 0..12 {
            let mut components = frame.components();
            components[index] += 0.25;
            let changed = CFrame::from_parts(
                Vec3::new(components[0], components[1], components[2]),
                [
                    [components[3], components[4], components[5]],
                    [components[6], components[7], components[8]],
                    [components[9], components[10], components[11]],
                ],
            );
            assert_ne!(digest(frame), digest(changed), "component {index}");
        }
        let mut negative_zero = CFrame::IDENTITY;
        negative_zero.position.x = -0.0;
        assert_ne!(digest(CFrame::IDENTITY), digest(negative_zero));
    }

    #[test]
    fn server_only_metadata_is_enforced_before_source_special_cases() {
        let mut def = prop();
        def.repl = Repl::ServerOnly;
        assert!(!property_allowed("Part", &def));
        assert!(!property_allowed("LocalScript", &def));
        def.repl = Repl::Replicated;
        def.hidden = true;
        assert!(property_allowed("Part", &def)); // UI/Luau hidden is not confidential.
    }

    #[test]
    fn adding_or_reordering_properties_and_changing_inheritance_changes_hash() {
        let single = hash_schema(&[base(prop())]);
        let mut extra = prop();
        extra.name = "Extra";
        let pair = ClassDef {
            name: "Fixture",
            superclass: None,
            props: Box::leak(vec![prop(), extra].into_boxed_slice()),
        };
        let pair_hash = hash_schema(&[pair]);
        assert_ne!(single, pair_hash);
        let mut extra = prop();
        extra.name = "Extra";
        let reversed = ClassDef {
            name: "Fixture",
            superclass: None,
            props: Box::leak(vec![extra, prop()].into_boxed_slice()),
        };
        assert_ne!(pair_hash, hash_schema(&[reversed]));
        let schema = |parent| {
            [
                base(prop()),
                ClassDef {
                    name: "OtherBase",
                    superclass: None,
                    props: &[],
                },
                ClassDef {
                    name: "Derived",
                    superclass: Some(parent),
                    props: &[],
                },
            ]
        };
        assert_ne!(
            hash_schema(&schema("Fixture")),
            hash_schema(&schema("OtherBase"))
        );
    }
}
