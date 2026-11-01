# #461 method calling convention (step 3 of the class-system migration)

Branch `studio/461-call-convention`. Design: `docs/design/class-system.md` (sections 3.2, 5, 8, 13.2, 14 row #461).

## Goal
Class methods are written once, with no knowledge of Luau. `ScriptValue`, `MethodCall`, `CallOutcome`, `ApiError` and its
formatter, `Host` method registration and its completeness test, the `HostApi` trait. No script sees any of this yet.

## Done
- `crates/metallic_core/src/classes/call.rs`: `ScriptValue`, `MethodCall` and accessors, `CallContext`, `Side`, `CallOutcome`,
  `WaitRequest`, `SignalKey`, `FunctionRef`, `EnumItemRef`, `ApiError`, `ApiErrorKind`, `TypeName`, `check_arguments`, `conforms`.
- `crates/metallic_core/src/classes/host_api.rs`: `HostApi`, `member_of`, `MethodDispatch`, `HostMethods`, `HostCall`.
- `desc.rs`: `MethodImpl::Native`, `MethodDesc::native`, `.variadic`, `.yields`. `replication.rs`: fingerprint byte 0 is `Native`.
- `crates/metallic_script/src/api/host_methods.rs`: the empty table, `HostState`, `NOT_MIGRATED_HOST_METHODS`. Not wired to Luau.
- Tests: `crates/metallic_core/tests/call_convention.rs`, `crates/metallic_script/tests/host_methods.rs`, one unit test in
  `replication.rs`. Docs: `ARCHITECTURE.md` "How to write a method", `call.rs` module docs with a compiled example.

## Next
Nothing in this issue once CI is green on the PR. #462 is the first real user; see "What #462 needs".

## Where the design differs from the issue text (the design was followed)
- `MethodFn` returns `Result<CallOutcome, ApiError>`, not `Result<ScriptValue, ApiError>`: yielding is a return value.
- One formatter in core, so every language prints the same text; the issue has each language format it.
- Argument checks run in the host path (`MethodDispatch`), below every binding, not in "the binding".
- Registration is by class and method name, resolved once to a `DeclId`, not "by `MethodId`".
- `ScriptValue` also has `Function` and `Tuple`.

## Decisions where the design left room
- `Handle` is an alias of `InstanceId`. A binding converts with `to_bits`/`from_bits` at the language edge.
- `EnumItemRef` holds names, not `EnumId` + item index: `EnumDesc` arrives with #469, so an id has nothing to index. Its fields
  are private so #469 can change it.
- `Ty::Enum` refuses a plain string, as the design's example text shows (`expected Enum.NormalId, got string`). Luau enums are
  strings until #469, so until then a method taking an enum should declare `Ty::String` or `.manual()`.
- `HostMethods<S>` and `HostCall<'a, S>` are generic over the host's state, so core names no script type.
  `metallic_script` gives them its `S` (`HostState`, empty for now) and the design's names by alias.
- `NOT_MIGRATED_HOST_METHODS` holds `PVInstance.GetPivot`/`PivotTo`, declared `Host` by #460 and still in `api/pivot.rs` until
  #465. The rule also fails on a stale entry, so the list can only shrink. A bridge body would have been dead code.
- `ApiError.member` is a `Cow`: `NotAMember` carries a name a script chose. `NotCreatable` holds the name asked for, for the same
  reason. Argument `index` is `u32`, one-based, what the text prints.
- New kind `TooManyArguments` (design 5.5 has the check, 5.3 no variant): `Part:Resize: expected at most 1 argument, got 2`.
  `MissingArgument` prints `... argument #1 expected T, got nothing`.
- `ApiError.class` is the instance's class (design's `Part:Resize`; `Resize` is a `BasePart` member).
- `got` for `Int` and `Number` is both `number`: a Luau number must not change its name with its value.
- A `.manual()` parameter is skipped by the count check too, because `PivotTo()` with no argument must keep its own text.
- A stale handle as an argument where an instance is declared gives `Destroyed`; nested in a list it gives `BadArgument`.
- `Destroyed.name` from core is `Instance` (`UNNAMED_INSTANCE`): a dead instance has no name in the World. Luau's `name_hint`
  gives today's text, see below.
- `MethodDispatch` errors when a method not declared `yields` returns `Wait`, so a wrong declaration is seen at once.
- `MethodDispatch` takes a `Registry` and finds the instance's class by name, so a test can run fixture methods on real instances
  without touching the real registry (`World::create` only knows the real one). `member_of` is shared with property access.
- **Context is not enforced** (#468): `CallContext` and `member.context` reach the place, marked in `MethodDispatch::call`.
  `WrongContext` and its two texts exist; the class-scoped `context_message` shape is #468's.
- `MethodCall` has no `signals` field and no `fire` yet (#463). `WaitRequest::Signal` and `SignalKey` exist, since section 5.4
  and section 6 fix their shape.
- `prelude.rs` allows an unused `call::*` until #465 gives a class file a `Native` method.

## Registry hash
Unchanged: no built-in class has a `Native` method, and `Host` and `Luau` keep bytes 1 and 2. A new unit test holds that the
byte depends on where the body lives, never on which function.

## Perf
`World::get`/`set` and the registry lookups were not touched, so `perf_ci` was not run. `Registry` and `resolve.rs` are unchanged.

## What #462 needs
- Implement `HostApi`; `call_method` builds a `MethodDispatch` with `classes::registry()`, `metallic_script::host_methods()` (build
  once, it walks every member), its `HostState` (fill it with what host bodies need; `ApiState` is the likely source) and a
  `CallContext` (`ScriptRole`/VM side to `Side`). Keep the `args` buffer per VM: `args` is a borrowed slice so a call allocates no list.
- Convert at the edge: Luau whole numbers to `Int` or `Number` as design 5.1 says; both are accepted either way.
- Rewrite `ApiErrorKind::Destroyed { name }` from the receiver's `name_hint` to keep `<Name> has been destroyed` (`read_probe.rs`
  asserts `Part has been destroyed`). Core only knows `Instance`.
- `resolve_member` should look through `ResolvedClass::member`; `member_of` already refuses hidden members at the call.
- **Extra arguments:** today most Luau methods ignore them; `check_arguments` refuses them when not variadic, as design 5.5 says.
  This reaches scripts once #465-#467 migrate methods, so each migration should check its method, and declare `.variadic(Ty::Any)`
  where the old behaviour must stay (that changes `registry_hash` on purpose).
- `WaitRequest` needs its trampoline in Luau (design 5.4); `CallOutcome::Wait` from a `yields` method is what it gets.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_core -j 4 --test call_convention
cargo test -p metallic_script -j 4 --test host_methods
```

## Open questions
None blocking.

## Last check results
See the PR body once checks have run on the final head.
