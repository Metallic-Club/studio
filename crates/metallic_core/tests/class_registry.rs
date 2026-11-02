//! The class registry's rules: discovery, resolution checks, ids, flags, and that the migration
//! from `classes.rs` (#460) changed no answer. Fixture class sets go through the public `resolve`.
use metallic_core::classes::{
    self, ClassDesc, ClassFlags, ClassId, Editor, MemberContext, MemberId, MemberKind, MethodDesc,
    Param, PropDesc, PropertyIndex, Registry, ResolvedClass, SchemaError, Section, SignalDesc, Ty,
    VisibleWhen, registry,
};
use metallic_core::{CoreError, Value, ValueType, World, load_scene};

const BOOL: PropDesc = PropDesc::new("P", ValueType::Bool, || Value::Bool(false));

static BASE: ClassDesc = ClassDesc::new("Base", None)
    .props(&[BOOL])
    .methods(&[MethodDesc::host("M")
        .params(&[Param::new("x", Ty::Number)])
        .returns(Ty::Bool)])
    .signals(&[&FIRED]);
static FIRED: SignalDesc = SignalDesc::new("Fired");

fn resolved(classes: &[&'static ClassDesc]) -> Registry {
    classes::resolve(classes).unwrap()
}

fn error(classes: &[&'static ClassDesc]) -> SchemaError {
    classes::resolve(classes).unwrap_err()
}

fn class<'a>(registry: &'a Registry, name: &str) -> &'a ResolvedClass {
    registry.class_by_name(name).unwrap()
}

// ---- Discovery and every built-in class ------------------------------------------------------

#[test]
fn every_discovered_class_resolves() {
    assert_eq!(classes::registry_error(), None);
    let classes = classes::registry_classes();
    assert_eq!(classes.len(), registry().classes().len());
    assert!(classes.len() >= 51, "discovery lost classes");
    for (index, class) in classes.iter().enumerate() {
        let found = classes::class_def(class.name).unwrap();
        assert!(std::ptr::eq(found, *class), "{}", class.name);
        let id = u16::try_from(index).unwrap();
        assert_eq!(classes::class_index(class.name), Some(id));
        assert!(std::ptr::eq(classes::class_at(id).unwrap(), *class));
    }
    assert!(classes::class_index("NotAClass").is_none());
    assert!(classes::class_def("NotAClass").is_none());
    assert!(classes::class_def("part").is_none());
    assert!(classes::class_at(u16::try_from(classes.len()).unwrap()).is_none());
}

#[test]
fn class_order_is_sorted_by_name() {
    let names: Vec<&str> = classes::registry_classes()
        .iter()
        .map(|class| class.name)
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

#[test]
fn every_superclass_exists_and_every_chain_reaches_instance_without_a_cycle() {
    let root = registry().class_id("Instance").unwrap();
    for class in registry().classes() {
        match class.desc.superclass {
            None => assert_eq!(class.name(), "Instance"),
            Some(superclass) => assert!(
                classes::class_def(superclass).is_some(),
                "{} names {superclass}",
                class.name()
            ),
        }
        assert_eq!(class.chain[0], class.id);
        assert_eq!(class.chain.last(), Some(&root), "{}", class.name());
        assert!(class.chain.len() <= registry().classes().len());
        let mut unique = class.chain.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), class.chain.len(), "{} repeats", class.name());
    }
}

#[test]
fn no_class_redeclares_an_inherited_property() {
    for class in classes::registry_classes() {
        let names: Vec<&str> = classes::all_properties(class.name)
            .iter()
            .map(|d| d.name)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "{} repeats", class.name);
    }
}

#[test]
fn every_class_is_described_and_every_default_has_the_declared_type() {
    for class in classes::registry_classes() {
        assert!(!class.category.is_empty(), "{} category", class.name);
        assert!(!class.description.is_empty(), "{} description", class.name);
        for prop in classes::all_properties(class.name) {
            let default = classes::default_value_ref(class.name, prop.name).unwrap();
            assert_eq!(
                default.value_type(),
                prop.ty,
                "{}.{}",
                class.name,
                prop.name
            );
        }
    }
}

// ---- The prefix rule --------------------------------------------------------------------------

#[test]
fn inherited_members_keep_their_ids_in_every_subclass() {
    for class in registry().classes() {
        for &ancestor in &class.chain {
            let ancestor = registry().class(ancestor).unwrap();
            assert!(ancestor.members.len() <= class.members.len());
            for (index, member) in ancestor.members.iter().enumerate() {
                let inherited = &class.members[index];
                assert_eq!(member.name(), inherited.name(), "{}", class.name());
                assert_eq!(member.owner, inherited.owner, "{}", class.name());
                if let (
                    MemberKind::Property { index: a, .. },
                    MemberKind::Property { index: b, .. },
                ) = (member.kind, inherited.kind)
                {
                    assert_eq!(a, b, "{} {}", class.name(), member.name());
                }
            }
        }
    }
    let part = class(registry(), "SpawnLocation");
    let base = class(registry(), "BasePart");
    assert_eq!(part.member("Size"), base.member("Size"));
    assert_eq!(
        part.property("Size").unwrap().1,
        base.property("Size").unwrap().1
    );
    assert_eq!(classes::property_index("Part", "Name"), Some(0));
}

#[test]
fn every_id_finds_its_member_and_the_name_table_misses_cleanly() {
    for class in registry().classes() {
        for (index, member) in class.members.iter().enumerate() {
            assert_eq!(class.member(member.name()), Some(MemberId(index as u16)));
        }
        for (index, prop) in class.props.iter().enumerate() {
            assert_eq!(
                class.property(prop.name).unwrap().1,
                PropertyIndex(index as u16)
            );
            assert!(std::ptr::eq(class.property(prop.name).unwrap().0, *prop));
        }
        assert_eq!(class.member("NotAMember"), None);
        assert_eq!(class.member(""), None);
    }
}

// ---- Migration: nothing changed ---------------------------------------------------------------

/// Recorded from `classes.rs` before #460: each line is a class, then every class it is a
/// subclass of, read by calling the old `is_subclass` for every pair.
const OLD_IS_SUBCLASS: &str = include_str!("fixtures/is_subclass_before_460.txt");
/// Recorded from `classes.rs` before #460: every effective property of every class.
const OLD_PROPERTIES: &str = include_str!("fixtures/properties_before_460.txt");

#[test]
fn is_subclass_gives_the_old_answer_for_every_pair_of_old_classes() {
    let table: Vec<(&str, Vec<&str>)> = OLD_IS_SUBCLASS
        .lines()
        .map(|line| {
            let (class, ancestors) = line.split_once(':').unwrap();
            (class, ancestors.split_whitespace().collect())
        })
        .collect();
    assert_eq!(table.len(), 43);
    let mut pairs = 0;
    for (class, ancestors) in &table {
        for (of, _) in &table {
            assert_eq!(
                classes::is_subclass(class, of),
                ancestors.contains(of),
                "is_subclass({class}, {of})"
            );
            pairs += 1;
        }
    }
    assert_eq!(pairs, 43 * 43);
}

/// Line format of the fixture; `Value`'s `Debug` output is the default's text.
fn property_line(class: &str, index: usize, prop: &PropDesc) -> String {
    let range = prop.range.map_or("-".to_string(), |r| {
        format!(
            "{:?}..{:?}{}",
            r.min,
            r.max,
            if r.wrap { " wrap" } else { "" }
        )
    });
    let allowed = prop
        .allowed
        .map_or("-".to_string(), |values| values.join(","));
    format!(
        "{class}|{index}|{}|{}|{}|{:?}|{range}|{allowed}|{}|{:?}",
        prop.name,
        prop.ty.name(),
        prop.hidden,
        prop.repl,
        prop.normalize_schema_id.unwrap_or("-"),
        classes::default_value_ref(class, prop.name).unwrap(),
    )
}

/// Classes and properties added later are fine: the old ones must still be there, unchanged and in
/// the same relative order. Only the numeric index may move. Changing a default on purpose means
/// changing its line here, with a reason in the PR.
#[test]
fn every_old_property_kept_its_type_default_checks_and_order() {
    let without_index = |line: &str| {
        let mut fields = line.splitn(3, '|');
        let (class, _, rest) = (fields.next(), fields.next(), fields.next());
        (class.unwrap().to_owned(), rest.unwrap().to_owned())
    };
    let mut current: Vec<(String, String)> = Vec::new();
    for class in classes::registry_classes() {
        for (index, prop) in classes::all_properties(class.name).iter().enumerate() {
            current.push(without_index(&property_line(class.name, index, prop)));
        }
    }
    let mut position = std::collections::HashMap::<String, usize>::new();
    let mut checked = 0;
    for line in OLD_PROPERTIES.lines() {
        let (class, rest) = without_index(line);
        let start = position.get(&class).copied().unwrap_or(0);
        let found = current
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, (c, r))| *c == class && *r == rest);
        let Some((at, _)) = found else {
            panic!("changed or moved: {line}");
        };
        position.insert(class, at + 1);
        checked += 1;
    }
    assert_eq!(checked, 261);
}

// ---- Hierarchy --------------------------------------------------------------------------------

#[test]
fn abstract_classes_are_inserted_above_the_existing_ones() {
    for (class, of) in [
        ("BasePart", "PVInstance"),
        ("Model", "PVInstance"),
        ("Part", "PVInstance"),
        ("SpawnLocation", "PVInstance"),
        ("Frame", "GuiObject"),
        ("GuiObject", "GuiBase2d"),
        ("TextLabel", "GuiObject"),
        ("GuiButton", "GuiBase2d"),
        ("ImageButton", "Frame"),
        ("ModuleScript", "LuaSourceContainer"),
        ("BaseScript", "LuaSourceContainer"),
        ("Script", "BaseScript"),
        ("LocalScript", "BaseScript"),
        ("LocalScript", "Script"),
        ("RemoteEvent", "BaseRemoteEvent"),
        ("UnreliableRemoteEvent", "BaseRemoteEvent"),
        ("DataModel", "ServiceProvider"),
    ] {
        assert!(classes::is_subclass(class, of), "{class} is a {of}");
    }
    assert!(!classes::is_subclass("ModuleScript", "BaseScript"));
    assert!(!classes::is_subclass("Workspace", "ServiceProvider"));
    assert!(!classes::is_subclass("RemoteFunction", "BaseRemoteEvent"));
    assert!(!classes::is_subclass("ScreenGui", "GuiBase2d"));
    let script = class(registry(), "Script");
    assert_eq!(
        registry().class(script.chain[1]).unwrap().name(),
        "BaseScript"
    );
    assert!(script.is_a(registry().class_id("Instance").unwrap()));
    assert!(!script.is_a(registry().class_id("Folder").unwrap()));
}

#[test]
fn a_method_declared_on_pvinstance_is_found_on_its_subclasses() {
    let owner = registry().class_id("PVInstance").unwrap();
    let id = class(registry(), "PVInstance").member("GetPivot").unwrap();
    for name in ["Part", "SpawnLocation", "Model"] {
        let class = class(registry(), name);
        assert_eq!(class.member("GetPivot"), Some(id), "{name}");
        let member = &class.members[usize::from(id.0)];
        assert_eq!(member.owner, owner);
        assert!(matches!(member.kind, MemberKind::Method { .. }));
    }
    assert_eq!(class(registry(), "Folder").member("GetPivot"), None);
}

#[test]
fn name_defaults_to_the_concrete_class_name_and_properties_are_not_methods() {
    assert_eq!(
        classes::default_value_ref("Part", "Name"),
        Some(&Value::String("Part".into()))
    );
    assert_eq!(
        classes::default_value("TextButton", "Name"),
        Some(Value::String("TextButton".into()))
    );
    // A method is a member, not a property: every property lookup misses it.
    assert!(classes::find_property("Part", "GetPivot").is_none());
    assert!(classes::default_value_ref("Part", "GetPivot").is_none());
    let mut world = World::new_with_services();
    let part = world.create("Part", None).unwrap();
    assert!(matches!(
        world.get(part, "GetPivot"),
        Err(CoreError::UnknownProperty { .. })
    ));
    assert_eq!(
        world
            .class_id(part)
            .and_then(|id| registry().class(id))
            .map(ResolvedClass::name),
        Some("Part")
    );
}

// ---- Flags ------------------------------------------------------------------------------------

#[test]
fn abstract_classes_and_services_are_not_creatable() {
    let mut abstract_names = Vec::new();
    let mut service_names = Vec::new();
    for class in classes::registry_classes() {
        if class.flags.contains(ClassFlags::ABSTRACT) {
            abstract_names.push(class.name);
            assert!(!class.is_creatable(), "{}", class.name);
        }
        if class.flags.contains(ClassFlags::SERVICE) {
            service_names.push(class.name);
            assert!(!class.is_creatable(), "{}", class.name);
        }
        if !class.flags.contains(ClassFlags::ABSTRACT) && !class.flags.contains(ClassFlags::SERVICE)
        {
            assert!(class.is_creatable(), "{} has no way in", class.name);
        }
    }
    for name in [
        "Instance",
        "BasePart",
        "PVInstance",
        "GuiBase2d",
        "GuiObject",
        "LuaSourceContainer",
        "BaseScript",
        "BaseRemoteEvent",
        "ServiceProvider",
        "DataModel",
    ] {
        assert!(abstract_names.contains(&name), "{name} is abstract");
    }
    // Every builtin root of a fresh world is a service.
    let world = World::new_with_services();
    for &root in world.roots() {
        let name = world.class_name(root).unwrap();
        assert!(service_names.contains(&name), "{name} is a service");
    }
    assert_eq!(service_names.len(), world.roots().len());
}

#[test]
fn the_world_and_scene_load_refuse_abstract_classes() {
    let mut world = World::new_with_services();
    let before = world.change_cursor();
    for name in [
        "Instance",
        "BasePart",
        "PVInstance",
        "GuiObject",
        "DataModel",
    ] {
        assert_eq!(
            world.create(name, None),
            Err(CoreError::AbstractClass(name.to_string()))
        );
    }
    assert_eq!(
        world.changes_since(before).0,
        metallic_core::Changes::Ids(Vec::new())
    );
    assert!(world.roots().iter().all(|&root| world.is_builtin(root)));
    let scene = "(version: 1, roots: [(class: \"PVInstance\")])";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("abstract.scene");
    std::fs::write(&path, scene).unwrap();
    assert_eq!(
        load_scene(&path).err(),
        Some(CoreError::AbstractClass("PVInstance".to_string()))
    );
}

#[test]
fn conflicting_flags_fail() {
    static BOTH: ClassDesc =
        ClassDesc::new("Both", None).flags(ClassFlags::CREATABLE.with(ClassFlags::SERVICE));
    static ABSTRACT_CREATABLE: ClassDesc = ClassDesc::new("AbstractCreatable", None)
        .flags(ClassFlags::ABSTRACT.with(ClassFlags::CREATABLE));
    static ABSTRACT_SERVICE: ClassDesc = ClassDesc::new("AbstractService", None)
        .flags(ClassFlags::ABSTRACT.with(ClassFlags::SERVICE));
    for class in [&BOTH, &ABSTRACT_CREATABLE, &ABSTRACT_SERVICE] {
        assert_eq!(error(&[class]), SchemaError::ConflictingFlags(class.name));
    }
}

// ---- Resolution rules -------------------------------------------------------------------------

#[test]
fn redeclaring_an_inherited_member_without_overrides_fails() {
    static PROP: ClassDesc = ClassDesc::new("Sub", Some("Base")).props(&[BOOL]);
    static METHOD: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).methods(&[MethodDesc::host("M")
            .params(&[Param::new("x", Ty::Number)])
            .returns(Ty::Bool)]);
    static SIGNAL: ClassDesc = ClassDesc::new("Sub", Some("Base")).signals(&[&FIRED]);
    // A different kind under an inherited name is still a redeclaration.
    static ACROSS_KINDS: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).methods(&[MethodDesc::host("P")]);
    for (sub, member) in [
        (&PROP, "P"),
        (&METHOD, "M"),
        (&SIGNAL, "Fired"),
        (&ACROSS_KINDS, "P"),
    ] {
        let found = error(&[&BASE, sub]);
        assert_eq!(
            found,
            SchemaError::RedeclaredMember {
                class: "Sub",
                member
            }
        );
        assert_eq!(
            found.to_string(),
            format!("class Sub redeclares inherited member {member} (add overrides())")
        );
    }
}

#[test]
fn an_override_replaces_in_place_and_the_superclass_keeps_its_own() {
    static SUB: ClassDesc = ClassDesc::new("Sub", Some("Base"))
        .props(&[
            PropDesc::new("P", ValueType::Bool, || Value::Bool(true)).overrides(),
            PropDesc::new("Q", ValueType::Number, || Value::Number(1.0)),
        ])
        .methods(&[MethodDesc::host("M")
            .params(&[Param::new("renamed", Ty::Number)])
            .returns(Ty::Bool)
            .overrides()
            .describe("Sub's M.")]);
    static DEEPER: ClassDesc = ClassDesc::new("Deeper", Some("Sub"));
    let registry = resolved(&[&BASE, &SUB, &DEEPER]);
    let (base, sub, deeper) = (
        class(&registry, "Base"),
        class(&registry, "Sub"),
        class(&registry, "Deeper"),
    );
    let m = base.member("M").unwrap();
    assert_eq!(sub.member("M"), Some(m));
    assert_eq!(deeper.member("M"), Some(m));
    let decl_of = |c: &ResolvedClass| match c.members[usize::from(m.0)].kind {
        MemberKind::Method { desc, decl } => (desc.description, decl),
        _ => panic!("M is a method"),
    };
    assert_eq!(decl_of(base).0, "");
    assert_eq!(decl_of(sub).0, "Sub's M.");
    assert_eq!(decl_of(deeper), decl_of(sub));
    assert_ne!(decl_of(base).1, decl_of(sub).1);
    assert_eq!(sub.members[usize::from(m.0)].owner, sub.id);
    assert_eq!(base.members[usize::from(m.0)].owner, base.id);

    assert_eq!(base.default_of("P"), Some(&Value::Bool(false)));
    assert_eq!(sub.default_of("P"), Some(&Value::Bool(true)));
    assert_eq!(deeper.default_of("P"), Some(&Value::Bool(true)));
    assert_eq!(sub.property("P").unwrap().1, PropertyIndex(0));
    assert_eq!(sub.property("Q").unwrap().1, PropertyIndex(1));
    // The new property comes after everything inherited, kinds included.
    assert_eq!(sub.member("Q"), Some(MemberId(3)));
}

#[test]
fn an_override_must_keep_the_shape() {
    static CHANGED_TYPE: ClassDesc =
        ClassDesc::new("Sub", Some("Base"))
            .props(&[PropDesc::new("P", ValueType::Number, || Value::Number(0.0)).overrides()]);
    static CHANGED_PARAM: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).methods(&[MethodDesc::host("M")
            .params(&[Param::new("x", Ty::String)])
            .returns(Ty::Bool)
            .overrides()]);
    static CHANGED_RETURN: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).methods(&[MethodDesc::host("M")
            .params(&[Param::new("x", Ty::Number)])
            .returns(Ty::Number)
            .overrides()]);
    static CHANGED_KIND: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).methods(&[MethodDesc::host("P").overrides()]);
    for (sub, member) in [
        (&CHANGED_TYPE, "P"),
        (&CHANGED_PARAM, "M"),
        (&CHANGED_RETURN, "M"),
        (&CHANGED_KIND, "P"),
    ] {
        assert_eq!(
            error(&[&BASE, sub]),
            SchemaError::OverrideChangesShape {
                class: "Sub",
                member
            }
        );
    }
}

#[test]
fn an_override_may_narrow_its_context_but_not_widen_it() {
    static NARROW: ClassDesc =
        ClassDesc::new("Narrow", Some("Base")).methods(&[MethodDesc::host("M")
            .params(&[Param::new("x", Ty::Number)])
            .returns(Ty::Bool)
            .context(MemberContext::Server)
            .overrides()]);
    static WIDER: ClassDesc =
        ClassDesc::new("Wider", Some("Narrow")).methods(&[MethodDesc::host("M")
            .params(&[Param::new("x", Ty::Number)])
            .returns(Ty::Bool)
            .context(MemberContext::Any)
            .overrides()]);
    let registry = resolved(&[&BASE, &NARROW]);
    let id = class(&registry, "Base").member("M").unwrap();
    assert_eq!(
        class(&registry, "Narrow").members[usize::from(id.0)].context,
        MemberContext::Server
    );
    assert_eq!(
        error(&[&BASE, &NARROW, &WIDER]),
        SchemaError::OverrideWidensContext {
            class: "Wider",
            member: "M"
        }
    );
}

#[test]
fn an_override_needs_something_to_override() {
    static SUB: ClassDesc =
        ClassDesc::new("Sub", Some("Base"))
            .props(&[PropDesc::new("New", ValueType::Bool, || Value::Bool(false)).overrides()]);
    assert_eq!(
        error(&[&BASE, &SUB]),
        SchemaError::OverrideWithoutBase {
            class: "Sub",
            member: "New"
        }
    );
}

#[test]
fn members_of_one_class_are_unique_across_kinds() {
    static TWICE: ClassDesc = ClassDesc::new("Twice", None).props(&[BOOL, BOOL]);
    static ACROSS: ClassDesc = ClassDesc::new("Across", None)
        .props(&[BOOL])
        .methods(&[MethodDesc::host("P")]);
    static ALIAS: ClassDesc = ClassDesc::new("Alias", None).props(&[
        BOOL,
        PropDesc::new("Q", ValueType::Bool, || Value::Bool(false)).aliases(&["P"]),
    ]);
    for (class, member) in [(&TWICE, "P"), (&ACROSS, "P"), (&ALIAS, "P")] {
        assert_eq!(
            error(&[class]),
            SchemaError::DuplicateMember {
                class: class.name,
                member
            }
        );
    }
}

#[test]
fn an_alias_resolves_to_the_same_member() {
    static OLD_NAME: ClassDesc =
        ClassDesc::new("Fixture", None)
            .props(
                &[PropDesc::new("New", ValueType::Bool, || Value::Bool(true)).aliases(&["Old"])],
            );
    let registry = resolved(&[&OLD_NAME]);
    let class = class(&registry, "Fixture");
    assert_eq!(class.member("Old"), class.member("New"));
    assert_eq!(class.property("Old").unwrap().0.name, "New");
    assert_eq!(class.default_of("Old"), Some(&Value::Bool(true)));
}

#[test]
fn defaults_change_an_inherited_property_for_the_class_and_below() {
    static SUB: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).defaults(&[("P", || Value::Bool(true))]);
    static DEEPER: ClassDesc = ClassDesc::new("Deeper", Some("Sub"));
    let registry = resolved(&[&BASE, &SUB, &DEEPER]);
    assert_eq!(
        class(&registry, "Base").default_of("P"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        class(&registry, "Sub").default_of("P"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        class(&registry, "Deeper").default_of("P"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        (class(&registry, "Sub").property("P").unwrap().0.default)(),
        Value::Bool(true)
    );
}

#[test]
fn defaults_must_name_an_inherited_property() {
    static UNKNOWN: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).defaults(&[("Missing", || Value::Bool(true))]);
    static OWN: ClassDesc = ClassDesc::new("Sub", Some("Base"))
        .props(&[PropDesc::new("Q", ValueType::Bool, || Value::Bool(false))])
        .defaults(&[("Q", || Value::Bool(true))]);
    static METHOD: ClassDesc =
        ClassDesc::new("Sub", Some("Base")).defaults(&[("M", || Value::Bool(true))]);
    for (sub, property) in [(&UNKNOWN, "Missing"), (&OWN, "Q"), (&METHOD, "M")] {
        assert_eq!(
            error(&[&BASE, sub]),
            SchemaError::DefaultWithoutProperty {
                class: "Sub",
                property
            }
        );
    }
}

#[test]
fn name_rule_writes_the_class_name_unless_defaults_set_it() {
    static ROOT: ClassDesc =
        ClassDesc::new("Root", None).props(&[PropDesc::new("Name", ValueType::String, || {
            Value::String(String::new())
        })]);
    static PLAIN: ClassDesc = ClassDesc::new("Plain", Some("Root"));
    static NAMED: ClassDesc = ClassDesc::new("Named", Some("Root"))
        .defaults(&[("Name", || Value::String("Custom".into()))]);
    static BELOW: ClassDesc = ClassDesc::new("Below", Some("Named"));
    let registry = resolved(&[&ROOT, &PLAIN, &NAMED, &BELOW]);
    let name = |of: &str| class(&registry, of).default_of("Name").cloned();
    assert_eq!(name("Root"), Some(Value::String("Root".into())));
    assert_eq!(name("Plain"), Some(Value::String("Plain".into())));
    assert_eq!(name("Named"), Some(Value::String("Custom".into())));
    assert_eq!(name("Below"), Some(Value::String("Custom".into())));
}

#[test]
fn hide_keeps_the_member_and_its_id_but_marks_it_for_the_class_and_below() {
    static SUB: ClassDesc = ClassDesc::new("Sub", Some("Base")).hide(&["P", "M"]);
    static DEEPER: ClassDesc = ClassDesc::new("Deeper", Some("Sub"));
    let registry = resolved(&[&BASE, &SUB, &DEEPER]);
    let base = class(&registry, "Base");
    let p = base.member("P").unwrap();
    let m = base.member("M").unwrap();
    assert!(!base.members[usize::from(p.0)].hidden);
    assert!(!base.property("P").unwrap().0.hidden);
    for name in ["Sub", "Deeper"] {
        let class = class(&registry, name);
        assert_eq!(class.member("P"), Some(p));
        assert!(class.members[usize::from(p.0)].hidden);
        assert!(class.members[usize::from(m.0)].hidden);
        // `hidden` on the descriptor is what the Luau binding and Studio already filter on.
        assert!(class.property("P").unwrap().0.hidden);
        assert_eq!(class.props[0].name, "P");
        assert_eq!(class.default_of("P"), Some(&Value::Bool(false)));
    }
}

#[test]
fn hide_survives_an_override_further_down() {
    static SUB: ClassDesc = ClassDesc::new("Sub", Some("Base")).hide(&["P"]);
    static DEEPER: ClassDesc =
        ClassDesc::new("Deeper", Some("Sub"))
            .props(&[PropDesc::new("P", ValueType::Bool, || Value::Bool(true)).overrides()]);
    let registry = resolved(&[&BASE, &SUB, &DEEPER]);
    let deeper = class(&registry, "Deeper");
    assert!(deeper.property("P").unwrap().0.hidden);
    assert_eq!(deeper.default_of("P"), Some(&Value::Bool(true)));
}

#[test]
fn hide_must_name_an_inherited_member() {
    static UNKNOWN: ClassDesc = ClassDesc::new("Sub", Some("Base")).hide(&["Missing"]);
    static ROOT: ClassDesc = ClassDesc::new("Root", None).props(&[BOOL]).hide(&["P"]);
    assert_eq!(
        error(&[&BASE, &UNKNOWN]),
        SchemaError::HideWithoutMember {
            class: "Sub",
            member: "Missing"
        }
    );
    assert_eq!(
        error(&[&ROOT]),
        SchemaError::HideWithoutMember {
            class: "Root",
            member: "P"
        }
    );
}

#[test]
fn malformed_sets_are_typed_errors() {
    static ORPHAN: ClassDesc = ClassDesc::new("Orphan", Some("Missing"));
    static NEEDS_ORPHAN: ClassDesc = ClassDesc::new("Child", Some("Orphan"));
    static A: ClassDesc = ClassDesc::new("A", Some("B"));
    static B: ClassDesc = ClassDesc::new("B", Some("A"));
    static SELF_LOOP: ClassDesc = ClassDesc::new("Own", Some("Own"));
    static BELOW_CYCLE: ClassDesc = ClassDesc::new("Z", Some("A"));
    static FIRST: ClassDesc = ClassDesc::new("Same", None);
    static SECOND: ClassDesc = ClassDesc::new("Same", None);
    assert_eq!(
        error(&[&NEEDS_ORPHAN, &ORPHAN]),
        SchemaError::UnknownSuperclass {
            class: "Orphan",
            superclass: "Missing"
        }
    );
    assert_eq!(error(&[&A, &B]), SchemaError::InheritanceCycle("A"));
    assert_eq!(error(&[&SELF_LOOP]), SchemaError::InheritanceCycle("Own"));
    assert_eq!(
        error(&[&A, &B, &BELOW_CYCLE]),
        SchemaError::InheritanceCycle("A")
    );
    assert_eq!(
        error(&[&FIRST, &SECOND]),
        SchemaError::DuplicateClass("Same")
    );
}

#[test]
fn more_than_65536_members_is_an_error() {
    let props = |count: usize| -> &'static [PropDesc] {
        (0..count)
            .map(|index| {
                let name: &'static str = Box::leak(format!("P{index}").into_boxed_str());
                PropDesc::new(name, ValueType::Bool, || Value::Bool(false))
            })
            .collect::<Vec<_>>()
            .leak()
    };
    let class = |count| -> &'static ClassDesc {
        Box::leak(Box::new(
            ClassDesc::new("Fixture", None).props(props(count)),
        ))
    };
    assert!(classes::resolve(&[class(65_536)]).is_ok());
    assert_eq!(
        classes::resolve(&[class(65_537)]).err(),
        Some(SchemaError::MemberIdOverflow("Fixture"))
    );
}

#[test]
fn ids_are_positions_in_name_order_and_decl_ids_are_dense() {
    static Z: ClassDesc = ClassDesc::new("Z", None)
        .methods(&[MethodDesc::host("One"), MethodDesc::host("Two")])
        .signals(&[&FIRED]);
    static A: ClassDesc = ClassDesc::new("A", Some("Z")).methods(&[MethodDesc::host("Three")]);
    let registry = resolved(&[&Z, &A]);
    assert_eq!(registry.class_id("A"), Some(ClassId(0)));
    assert_eq!(registry.class_id("Z"), Some(ClassId(1)));
    let mut decls: Vec<u32> = Vec::new();
    for class in registry.classes() {
        for member in &class.members {
            if member.owner == class.id
                && let MemberKind::Method { decl, .. } | MemberKind::Callback { decl, .. } =
                    member.kind
            {
                decls.push(decl.0);
            }
            if member.owner == class.id
                && let MemberKind::Signal { decl, .. } = member.kind
            {
                decls.push(decl.0);
            }
        }
    }
    decls.sort_unstable();
    assert_eq!(decls, [0, 1, 2, 3]);
}

// ---- What Studio reads (#473) ------------------------------------------------------------------

#[test]
fn every_class_and_property_a_creator_sees_has_a_description() {
    for class in registry().classes() {
        assert!(
            !class.desc.description.is_empty(),
            "class {} has no describe(...)",
            class.name()
        );
        for prop in class.props.iter().filter(|prop| !prop.hidden) {
            assert!(
                !prop.description.is_empty(),
                "{}.{} has no describe(...): Studio shows it as a tooltip",
                class.name(),
                prop.name
            );
        }
    }
}

#[test]
fn insert_lists_hold_listed_creatable_classes_with_each_category_together() {
    const FLAGS: ClassFlags = ClassFlags::CREATABLE;
    static ROOT: ClassDesc = ClassDesc::new("Root", None).flags(ClassFlags::ABSTRACT);
    static SECOND: ClassDesc = ClassDesc::new("A", Some("Root"))
        .flags(FLAGS)
        .category("Second")
        .insert_order(20);
    static FIRST: ClassDesc = ClassDesc::new("B", Some("Root"))
        .flags(FLAGS)
        .category("First")
        .insert_order(10);
    // Late in its category's order, but its category comes first.
    static FIRST_LATE: ClassDesc = ClassDesc::new("C", Some("Root"))
        .flags(FLAGS)
        .category("First")
        .insert_order(30);
    // No order: last in its category, and a new category goes after the ordered ones.
    static SECOND_NEW: ClassDesc = ClassDesc::new("D", Some("Root"))
        .flags(FLAGS)
        .category("Second");
    static OTHER_NEW: ClassDesc = ClassDesc::new("E", Some("Root"))
        .flags(FLAGS)
        .category("Third");
    static OTHER_NEW_TOO: ClassDesc = ClassDesc::new("F", Some("Root"))
        .flags(FLAGS)
        .category("Third");
    static HIDDEN: ClassDesc = ClassDesc::new("G", Some("Root"))
        .flags(FLAGS)
        .category("First")
        .no_insert();
    static NOT_CREATABLE: ClassDesc = ClassDesc::new("H", Some("Root"))
        .category("First")
        .insert_order(1);
    let registry = resolved(&[
        &ROOT,
        &SECOND,
        &FIRST,
        &FIRST_LATE,
        &SECOND_NEW,
        &OTHER_NEW,
        &OTHER_NEW_TOO,
        &HIDDEN,
        &NOT_CREATABLE,
    ]);
    let names: Vec<&str> = registry
        .insertable()
        .iter()
        .map(|class| class.name)
        .collect();
    assert_eq!(names, ["B", "C", "A", "D", "E", "F"]);
}

#[test]
fn visible_when_reads_a_property_of_the_class() {
    const NAMED: Editor = Editor::new().visible_when(VisibleWhen::Is("P", true));
    static INHERITED: ClassDesc =
        ClassDesc::new("Sub", Some("Base"))
            .props(&[PropDesc::new("Q", ValueType::Bool, || Value::Bool(false)).editor(NAMED)]);
    resolved(&[&BASE, &INHERITED]);
    static TYPO: ClassDesc =
        ClassDesc::new("Sub", Some("Base"))
            .props(&[PropDesc::new("Q", ValueType::Bool, || Value::Bool(false))
                .editor(Editor::new().visible_when(VisibleWhen::Is("Nope", true)))]);
    assert_eq!(
        error(&[&BASE, &TYPO]),
        SchemaError::VisibleWhenWithoutProperty {
            class: "Sub",
            property: "Q",
            reads: "Nope",
        }
    );
}

#[test]
fn visible_when_holds_only_for_the_values_it_names() {
    let on = VisibleWhen::Is("Lit", true);
    assert_eq!(on.property(), "Lit");
    assert!(on.holds(Some(&Value::Bool(true))));
    assert!(!on.holds(Some(&Value::Bool(false))));
    assert!(!on.holds(Some(&Value::Number(1.0))), "another type");
    assert!(!on.holds(None), "no value");
    let kinds = VisibleWhen::OneOf("Kind", &["Ball", "Cylinder"]);
    assert!(kinds.holds(Some(&Value::String("Ball".into()))));
    assert!(!kinds.holds(Some(&Value::String("Block".into()))));
    assert!(!kinds.holds(Some(&Value::Bool(true))));
}

#[test]
fn defaults_and_hide_keep_a_property_s_editor_metadata() {
    const SHOWN: Editor = Editor::section(Section::PHYSICS).label("Shown");
    static PARENT: ClassDesc =
        ClassDesc::new("Parent", None).props(&[PropDesc::new("P", ValueType::Bool, || {
            Value::Bool(false)
        })
        .editor(SHOWN)
        .describe("Text.")]);
    static CHANGED: ClassDesc =
        ClassDesc::new("Changed", Some("Parent")).defaults(&[("P", || Value::Bool(true))]);
    let registry = resolved(&[&PARENT, &CHANGED]);
    let (prop, _) = class(&registry, "Changed").property("P").unwrap();
    assert_eq!(prop.editor, SHOWN);
    assert_eq!(prop.description, "Text.");
}
