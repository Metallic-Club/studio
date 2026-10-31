//! Diagnostic getter probes use the real API and precompiled bounded Lua functions.

use super::*;
use crate::api::{HostShared, install};
use crate::{Context, OutputSink, ScriptVm};
use metallic_core::{Vec3, World};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    calls: usize,
    requested_bytes: usize,
}

thread_local! {
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { calls: 0, requested_bytes: 0 }) };
    static OWNED_BASELINE: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn owned_baseline() -> bool {
    OWNED_BASELINE.with(Cell::get)
}

fn with_baseline<T>(owned: bool, operation: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            OWNED_BASELINE.set(self.0);
        }
    }
    let restore = Restore(OWNED_BASELINE.replace(owned));
    let result = operation();
    drop(restore);
    result
}

struct Counter;

fn record(bytes: usize) {
    if RECORDING.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|counts| {
            let old = counts.get();
            counts.set(Counts {
                calls: old.calls + 1,
                requested_bytes: old.requested_bytes + bytes,
            });
        });
    }
}

unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: preserve System's allocator contract and only observe successful requests.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the same layout forwarded to System.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer/layout come from the GlobalAlloc caller, unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: preserve the caller's System realloc contract, including allocation failure.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: Counter = Counter;

struct Recording;

impl Drop for Recording {
    fn drop(&mut self) {
        RECORDING.set(false);
    }
}

fn count<T>(operation: impl FnOnce() -> T) -> (T, Counts) {
    COUNTS.set(Counts::default());
    RECORDING.set(true);
    let recording = Recording;
    let result = operation();
    drop(recording);
    (result, COUNTS.get())
}

struct Silent;

impl OutputSink for Silent {
    fn print(&self, _: &str) {}
    fn warn(&self, _: &str) {}
    fn error(&self, _: &str) {}
}

struct Fixture {
    vm: ScriptVm,
    world: Rc<RefCell<World>>,
    state: Rc<ApiState>,
    id: InstanceId,
    part: AnyUserData,
}

impl Fixture {
    fn new() -> Self {
        let world = Rc::new(RefCell::new(World::new_with_services()));
        let id = {
            let mut world = world.borrow_mut();
            let parent = world.root("Workspace");
            let id = world.create("Part", parent).unwrap();
            world
                .set(
                    id,
                    "Position",
                    CoreValue::Vector3(Vec3::new(1.000000000001, 2.5, -3.0)),
                )
                .unwrap();
            id
        };
        let output = Rc::new(Silent);
        let vm = ScriptVm::with_output(Context::Server, output.clone()).unwrap();
        let state = install(&vm, world.clone(), Rc::new(HostShared::default()), output).unwrap();
        let part = state.wrap(vm.lua(), id).unwrap();
        Self {
            vm,
            world,
            state,
            id,
            part,
        }
    }

    fn function(&self, source: &str) -> Function {
        self.vm
            .lua()
            .load(source)
            .set_name("=member_probe")
            .eval()
            .unwrap()
    }

    fn call(&self, function: &Function, subject: Value, reads: u32) -> Value {
        self.vm
            .call_with_budget(function, (subject, reads))
            .unwrap()
    }

    fn finish(self) {
        self.vm.shutdown();
    }
}

#[test]
fn valid_member_keys_do_not_allocate_owned_rust_strings() {
    let lua = Lua::new();
    let key = Value::String(lua.create_string("Anchored").unwrap());
    for _ in 0..64 {
        black_box(member_name(&key).unwrap());
    }
    let (_, counts) = count(|| {
        for _ in 0..10_000 {
            let member = member_name(&key).unwrap();
            let text: &str = member.as_ref();
            black_box(text);
        }
    });
    eprintln!("valid_member_keys_10000: {counts:?}");
    assert_eq!(counts.calls, 0);
    assert_eq!(counts.requested_bytes, 0);
}

#[test]
fn member_name_result_keeps_its_string_storage_alive() {
    let member = {
        let lua = Lua::new();
        let key = Value::String(lua.create_string("名前").unwrap());
        member_name(&key).unwrap()
    };
    let text: &str = member.as_ref();
    assert_eq!(text, "名前");
}

#[test]
fn non_string_keys_remain_rejected_without_numeric_coercion() {
    let lua = Lua::new();
    for key in [
        Value::Nil,
        Value::Boolean(true),
        Value::Number(1.25),
        Value::Table(lua.create_table().unwrap()),
    ] {
        let error = member_name(&key).err().unwrap().to_string();
        assert!(
            error.contains("an Instance member name must be a string"),
            "{error}"
        );
        assert!(error.contains(key.type_name()), "{error}");
    }
}

#[test]
fn invalid_utf8_keeps_lossy_child_lookup_and_assignment_errors() {
    let fixture = Fixture::new();
    {
        let mut world = fixture.world.borrow_mut();
        let child = world.create("Folder", Some(fixture.id)).unwrap();
        world
            .set(child, "Name", CoreValue::String("開閉\u{fffd}".into()))
            .unwrap();
    }
    let function = fixture.function(
        r#"return function(p)
        assert(p['開閉\255'].Name == '開閉�')
        local ok, err = pcall(function() p['\255'] = true end)
        assert(not ok and string.find(err, '� is not a valid member of Part', 1, true))
    end"#,
    );
    fixture
        .vm
        .call_with_budget::<()>(&function, fixture.part.clone())
        .unwrap();
    fixture.finish();
}

#[test]
fn getter_precision_and_existing_member_errors_are_preserved() {
    let fixture = Fixture::new();
    let function = fixture.function(
        r#"return function(p)
        assert(p.Position.X > 1 and p.Position.X < 1.000000000002)
        local ok, err = pcall(function() return p.DoesNotExist end)
        assert(not ok and string.find(err, 'DoesNotExist is not a valid member of Part', 1, true))
        ok, err = pcall(function() p.ClassName = 'Changed' end)
        assert(not ok and string.find(err, 'ClassName is read-only', 1, true))
        p:Destroy()
        local name
        ok, name = pcall(function() return p.Name end)
        assert(ok and name == 'Part')
        ok, err = pcall(function() p.Parent = nil end)
        assert(not ok and string.find(err, 'Parent property of Part is locked', 1, true))
        ok, err = pcall(function() return p[1.25] end)
        assert(not ok and string.find(err, 'member name must be a string', 1, true))
    end"#,
    );
    fixture
        .vm
        .call_with_budget::<()>(&function, fixture.part.clone())
        .unwrap();
    fixture.finish();
}

#[test]
#[ignore = "diagnostic release probe; no wall-time CI threshold"]
fn property_getter_probe() {
    const READS: u32 = 1_000;
    const CALLS: usize = 10;
    const SAMPLES: usize = 20;
    let fixture = Fixture::new();
    let table = fixture.vm.lua().create_table().unwrap();
    table.set("Anchored", false).unwrap();
    let scalar =
        fixture.function("return function(p,n) local v for i=1,n do v=p.Anchored end return v end");
    let vector =
        fixture.function("return function(p,n) local v for i=1,n do v=p.Position end return v end");
    eprintln!(
        "getter probe: precompiled; batch={READS}; samples={SAMPLES}; calls_per_sample={CALLS}; instrumented test allocator; machine load uncontrolled"
    );
    for (name, function, subject) in [
        (
            "scalar_instance",
            &scalar,
            Value::UserData(fixture.part.clone()),
        ),
        (
            "vector_instance",
            &vector,
            Value::UserData(fixture.part.clone()),
        ),
        ("scalar_table_reference", &scalar, Value::Table(table)),
    ] {
        for _ in 0..64 {
            black_box(fixture.call(function, subject.clone(), READS));
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = Instant::now();
            for _ in 0..CALLS {
                black_box(fixture.call(function, subject.clone(), READS));
            }
            samples.push(
                start.elapsed().as_nanos() as f64 / (u128::from(READS) * CALLS as u128) as f64,
            );
        }
        samples.sort_by(f64::total_cmp);
        let (_, allocations) = count(|| {
            black_box(fixture.call(function, subject.clone(), READS));
        });
        eprintln!(
            "{name}: p50_ns/read={:.1} p95_ns/read={:.1}; batch_allocations={allocations:?}; lua_heap_after_bytes={}",
            samples[SAMPLES / 2],
            samples[SAMPLES * 95 / 100 - 1],
            fixture.vm.lua().used_memory()
        );
    }
    let key = Value::String(fixture.vm.lua().create_string("Anchored").unwrap());
    let (_, keys) = count(|| {
        for _ in 0..10_000 {
            black_box(member_name(&key).unwrap());
        }
    });
    eprintln!("key_conversion_only_10000: {keys:?}");
    black_box(&fixture.state);
    fixture.finish();
}

#[test]
#[ignore = "paired diagnostic release probe; no wall-time CI threshold"]
fn paired_property_getter_probe() {
    const READS: u32 = 1_000;
    const CALLS: usize = 10;
    const SAMPLES: usize = 40;
    let fixture = Fixture::new();
    let scalar =
        fixture.function("return function(p,n) local v for i=1,n do v=p.Anchored end return v end");
    let vector =
        fixture.function("return function(p,n) local v for i=1,n do v=p.Position end return v end");
    let key = Value::String(fixture.vm.lua().create_string("Anchored").unwrap());
    eprintln!(
        "paired probe: one VM; alternating AB/BA; samples={SAMPLES}; batch={READS}; calls_per_sample={CALLS}; same test-only mode switch and allocator instrumentation; host load uncontrolled"
    );
    for (name, function) in [("scalar_instance", &scalar), ("vector_instance", &vector)] {
        for owned in [true, false] {
            with_baseline(owned, || {
                for _ in 0..64 {
                    black_box(fixture.call(function, Value::UserData(fixture.part.clone()), READS));
                }
            });
        }
        let mut owned_samples = Vec::with_capacity(SAMPLES);
        let mut borrowed_samples = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            for owned in if sample % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                let nanos = with_baseline(owned, || {
                    let start = Instant::now();
                    for _ in 0..CALLS {
                        black_box(fixture.call(
                            function,
                            Value::UserData(fixture.part.clone()),
                            READS,
                        ));
                    }
                    start.elapsed().as_nanos() as f64 / (u128::from(READS) * CALLS as u128) as f64
                });
                if owned {
                    owned_samples.push(nanos);
                } else {
                    borrowed_samples.push(nanos);
                }
            }
        }
        let mut ratios: Vec<f64> = owned_samples
            .iter()
            .zip(&borrowed_samples)
            .map(|(a, b)| b / a)
            .collect();
        ratios.sort_by(f64::total_cmp);
        for (mode, samples, owned) in [
            ("owned", &mut owned_samples, true),
            ("borrowed", &mut borrowed_samples, false),
        ] {
            samples.sort_by(f64::total_cmp);
            let (_, allocations) = with_baseline(owned, || {
                count(|| {
                    black_box(fixture.call(function, Value::UserData(fixture.part.clone()), READS));
                })
            });
            eprintln!(
                "paired {name}/{mode}: p50_ns/read={:.1} p95_ns/read={:.1}; allocations={allocations:?}",
                samples[SAMPLES / 2],
                samples[SAMPLES * 95 / 100 - 1]
            );
        }
        eprintln!(
            "{name}: paired median borrowed/owned ratio={:.3}",
            ratios[SAMPLES / 2]
        );
    }
    let mut owned_samples = Vec::with_capacity(SAMPLES);
    let mut borrowed_samples = Vec::with_capacity(SAMPLES);
    const CONVERSIONS: usize = 20_000;
    for sample in 0..SAMPLES {
        for owned in if sample % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let nanos = with_baseline(owned, || {
                let start = Instant::now();
                for _ in 0..CONVERSIONS {
                    let member = member_name(&key).unwrap();
                    black_box(member.as_ref());
                }
                start.elapsed().as_nanos() as f64 / CONVERSIONS as f64
            });
            if owned {
                owned_samples.push(nanos);
            } else {
                borrowed_samples.push(nanos);
            }
        }
    }
    for (mode, samples, owned) in [
        ("owned", &mut owned_samples, true),
        ("borrowed", &mut borrowed_samples, false),
    ] {
        samples.sort_by(f64::total_cmp);
        let (_, allocations) = with_baseline(owned, || {
            count(|| {
                for _ in 0..CONVERSIONS {
                    let member = member_name(&key).unwrap();
                    black_box(member.as_ref());
                }
            })
        });
        eprintln!(
            "paired key_conversion/{mode}: p50_ns/key={:.1} p95_ns/key={:.1}; allocations={allocations:?}",
            samples[SAMPLES / 2],
            samples[SAMPLES * 95 / 100 - 1]
        );
    }
    fixture.finish();
}
