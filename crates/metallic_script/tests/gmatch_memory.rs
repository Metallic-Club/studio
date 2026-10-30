//! Iterator retention must stay inside the same allocator budget as Luau strings.

use metallic_script::{Context, ScriptVm};
use mlua::{Function, Lua, Table, Value};

const MIB: usize = 1024 * 1024;

fn collect(lua: &Lua) {
    // The first collection finalizes unreachable Rust callbacks; the second can
    // collect Lua strings whose handles those callbacks just released.
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
}

fn retained_strings_hit_limit(large_pattern: bool) {
    let vm = ScriptVm::new(Context::Server).unwrap();
    let lua = vm.lua();
    collect(lua);
    lua.set_memory_limit(8 * MIB).unwrap();
    let iterators = lua.create_table().unwrap();
    lua.globals().set("retained", iterators.clone()).unwrap();
    let factory: Function = lua
        .globals()
        .get::<Table>("string")
        .unwrap()
        .get("gmatch")
        .unwrap();
    let mut bytes = vec![b'x'; MIB];
    let result = (|| -> mlua::Result<()> {
        // Different strings defeat Luau interning. Each source handle is moved
        // into the call and then survives only if the iterator retains it.
        for index in 1..=24u8 {
            bytes[0] = index;
            let large = lua.create_string(&bytes)?;
            let small = lua.create_string(b".")?;
            let iterator: Function = if large_pattern {
                factory.call((small, large))?
            } else {
                factory.call((large, small))?
            };
            iterators.set(index, iterator)?;
            collect(lua);
        }
        Ok(())
    })();
    let error = result.expect_err("retained distinct strings must exhaust the VM allocator");
    assert!(error.to_string().contains("not enough memory"), "{error}");

    // Releasing the iterators must release their subjects/patterns as well.
    lua.globals().set("retained", Value::Nil).unwrap();
    drop(iterators);
    collect(lua);
    assert!(
        lua.used_memory() < MIB,
        "{} bytes remain",
        lua.used_memory()
    );
    lua.load("assert(1 + 1 == 2)").exec().unwrap();
}

#[test]
fn distinct_retained_subjects_exhaust_vm_memory() {
    retained_strings_hit_limit(false);
}

#[test]
fn distinct_retained_patterns_exhaust_vm_memory() {
    retained_strings_hit_limit(true);
}

#[test]
fn shared_subject_is_counted_once_and_released_with_iterators() {
    let vm = ScriptVm::new(Context::Server).unwrap();
    let lua = vm.lua();
    collect(lua);
    lua.set_memory_limit(8 * MIB).unwrap();
    let baseline = lua.used_memory();
    // Host-driven bounded setup isolates memory accounting from execution-budget
    // timing. Scripts cannot invoke collection or change the VM limit themselves.
    lua.load(
        r#"
        local subject = string.rep('x', 1048576)
        retained = {}
        for i = 1, 64 do
            retained[i] = string.gmatch(subject, '()(%a)')
        end
        "#,
    )
    .exec()
    .unwrap();
    collect(lua);
    let retained = lua.used_memory();
    assert!(retained >= baseline + MIB, "subject escaped VM accounting");
    assert!(
        retained < baseline + 2 * MIB,
        "shared subject was duplicated"
    );
    lua.load(
        r#"
        for _, iterator in retained do
            local position, value = iterator()
            assert(position == 1 and value == 'x')
            position, value = iterator()
            assert(position == 2 and value == 'x')
        end
        local binary = string.gmatch(string.char(255, 0, 255), '()(.)')
        local a, b = binary()
        assert(a == 1 and b == string.char(255))
        a, b = binary()
        assert(a == 2 and b == string.char(0))
        a, b = binary()
        assert(a == 3 and b == string.char(255))
        assert(binary() == nil)
        retained = nil
        "#,
    )
    .exec()
    .unwrap();
    collect(lua);
    assert!(
        retained - lua.used_memory() >= MIB,
        "retained subject leaked"
    );
}
