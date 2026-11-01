# #459 Class system design

Status: design written (docs only); owner or conductor review pending. Branch: `studio/459-class-system-design`.

## Goal

Write `docs/design/class-system.md`: the target class system (one registry with inherited properties, methods,
signals and callbacks; language-neutral calling convention and host interface) that every follow-up issue of epic
#458 (#460-#474) implements. Decide points 1-12 of the issue with reasons and one example class file.

## Done

- `docs/design/class-system.md`: decisions in short, current problems with file:line pointers (main `e8fa980`), goals
  and non-goals, the model (ClassDesc, flags, members, inheritance rules, hierarchy changes, no member sets now),
  resolution and ids (prefix rule, `ClassId` in World nodes, sort-by-name class order, `registry_hash` v5, perf gate),
  calling convention (`ScriptValue`, `MethodCall`, `CallOutcome::Wait`, `ApiError`, host-implemented methods), signals,
  computed properties, execution context, enums and data types, Luau-implemented members, file layout with an
  illustrative `Beacon` class, generated outputs, "Every language" (handles, `HostApi`, linking by name, WIT shape,
  type mapping table, classes per language, errors, async, sandbox), migration plan mapped to #460-#474, files that go
  away, alternatives, open questions.
- Inputs read: issue #459 and #458, #460-#474, `classes.rs`, `replication.rs`, `api/instance.rs`, `signals.rs`,
  `services.rs`, `remote.rs`, `input.rs`, `host.rs`, `vm.rs`, the perf and research docs, the spec, and the Forge
  reference (ideas only; no code copied).
- `docs/STATUS.md` left unchanged: it has no list of design docs.
- No code changed; cargo not run (docs-only task).

## Next

- Owner or conductor review of the design; answers to section 17.
- After acceptance: move #460 to `status:ready`; adjust follow-up issue texts where the design differs from them
  (#462 and #468 keep today's error texts during migration; #470 uses a fixture class, `WaitForChild` goes native in
  #465; #474 WIT is a generic interface, Recommended; a Luau perf-workload PR lands before #462).

## Open questions (section 17 of the design)

1. Unify error texts after the migration (changes asserted test strings)?
2. Enum property reads return `EnumItem` (#469)?
3. WIT: generic id-based interface plus generated classes, or one resource per class?
4. `game` as a real `DataModel` instance?
5. Keep `TextLabel`/`GuiButton` under `Frame`?
