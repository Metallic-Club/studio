//! Registry benchmarks: building it, and the two lookups hot paths make (name to `PropId`, `is_a`).
//!
//! Run with `cargo bench -p metallic_core --bench registry`. Issue #13 sets the budget: building 100 classes takes
//! under 1 ms.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_core::{
    ClassDesc, ClassId, EnumDesc, MethodCall, MethodDesc, PropMeta, Registry, RegistryInput, ScriptError, ScriptValue,
    SignalDesc,
};

/// How many classes the synthetic hierarchy has.
const CLASSES: usize = 100;

/// Each class's parent is `(i - 1) / FAN_OUT`, giving a tree about four levels deep, like a real class hierarchy.
const FAN_OUT: usize = 4;

static ENUMS: &[EnumDesc] = &[EnumDesc::new("Mode", &["Off", "Low", "High"])];

fn noop(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(ScriptValue::Nil)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// 100 classes with 8 properties of mixed types, 2 methods and a signal each.
fn synthetic_classes() -> Vec<&'static ClassDesc> {
    (0..CLASSES)
        .map(|i| {
            let props: Vec<PropMeta> = vec![
                PropMeta::boolean(leak(format!("C{i}Enabled")), true),
                PropMeta::number(leak(format!("C{i}Brightness")), 1.0).range(0.0, 10.0),
                PropMeta::int(leak(format!("C{i}Count")), 3),
                PropMeta::string(leak(format!("C{i}Label")), "hello"),
                PropMeta::vector3(leak(format!("C{i}Size")), 1.0, 1.0, 1.0),
                PropMeta::cframe(leak(format!("C{i}CFrame"))),
                PropMeta::color3(leak(format!("C{i}Color")), 1.0, 0.5, 0.0),
                PropMeta::enumeration(leak(format!("C{i}Mode")), "Mode", "Low"),
            ];
            let methods = vec![
                MethodDesc::new(leak(format!("C{i}Run")), noop).returns_nothing(),
                MethodDesc::new(leak(format!("C{i}Stop")), noop).returns_nothing(),
            ];
            let signals = vec![SignalDesc::new(leak(format!("C{i}Changed")))];
            let parent = (i > 0).then(|| leak(format!("Class{}", (i - 1) / FAN_OUT)));
            let class = ClassDesc::new(leak(format!("Class{i}")), parent)
                .props(Box::leak(props.into_boxed_slice()))
                .methods(Box::leak(methods.into_boxed_slice()))
                .signals(Box::leak(signals.into_boxed_slice()));
            &*Box::leak(Box::new(class))
        })
        .collect()
}

fn registry(classes: &[&'static ClassDesc]) -> Registry {
    let input = RegistryInput {
        classes,
        enums: &[ENUMS],
        ..RegistryInput::default()
    };
    match Registry::build(input) {
        Ok(registry) => registry,
        Err(errors) => panic!("synthetic classes are invalid: {errors}"),
    }
}

fn class_id(registry: &Registry, name: &str) -> ClassId {
    match registry.class_id(name) {
        Some(id) => id,
        None => panic!("no class {name}"),
    }
}

fn benches(c: &mut Criterion) {
    let classes = synthetic_classes();
    let input = RegistryInput {
        classes: &classes,
        enums: &[ENUMS],
        ..RegistryInput::default()
    };
    c.bench_function("build_100_classes", |b| {
        b.iter_with_large_drop(|| Registry::build(black_box(input)))
    });

    let registry = registry(&classes);
    // The last class is one of the deepest: it inherits from three ancestors.
    let leaf = class_id(&registry, &format!("Class{}", CLASSES - 1));
    let root = class_id(&registry, "Class0");
    let sibling = class_id(&registry, "Class1");
    let depth = registry.class(leaf).map_or(0, |c| c.depth());
    println!("leaf depth {depth}, {} properties", registry.props(leaf).len());

    c.bench_function("prop_id_inherited", |b| {
        b.iter(|| registry.prop_id(black_box(leaf), black_box("C0Brightness")))
    });
    c.bench_function("prop_id_own", |b| {
        b.iter(|| registry.prop_id(black_box(leaf), black_box("C99Mode")))
    });
    c.bench_function("prop_id_missing", |b| {
        b.iter(|| registry.prop_id(black_box(leaf), black_box("NoSuchProperty")))
    });
    c.bench_function("is_a_true", |b| {
        b.iter(|| registry.is_a(black_box(leaf), black_box(root)))
    });
    c.bench_function("is_a_false", |b| {
        b.iter(|| registry.is_a(black_box(leaf), black_box(sibling)))
    });
}

criterion_group!(registry_benches, benches);
criterion_main!(registry_benches);
