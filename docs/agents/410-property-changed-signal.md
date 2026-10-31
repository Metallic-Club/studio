# #410 `Instance:GetPropertyChangedSignal(name)` and `Instance.Changed`

## Goal

Luau scripts can watch property changes: `inst:GetPropertyChangedSignal(name)` (no arguments) and `inst.Changed`
(property name). Real changes only, Rust and Luau writes alike, hidden properties never, Roblox error text for unknown
names, normal signal dispatch, writes inside handlers fire on a later dispatch, no cost for unwatched properties.

## Base

Branch `studio/410-property-changed-signal` from `origin/main` (`a0bb1bf`). Not pushed, no PR yet.

## Design (and why)

Two options were on the table: per-(instance, property) core `Signal`s connected while a Luau connection exists, or
reading the world's change journal at the host's dispatch point. The journal won:

- The core journal already records every real change (same-value and rejected writes are not journaled). It only
  lacked *which* property changed. Each entry now carries `Option<&'static PropDef>` (8 bytes, niche-optimised), set by
  `World::set`. New `World::property_changes_since(cursor)` yields `PropertyChange { id, prop, at }` per write, in
  order, not deduplicated; `None` on overflow / foreign cursor. `ChangeCursor` got `PartialOrd` (same world only).
- No handler runs inside `World::set`, so no re-entry into the World borrow and no recursion. The host calls
  `ApiState::deliver_property_changes` once per `step` (after inputs and GUI events, before Heartbeat). It snapshots the
  journal slice first, releases the borrow, then fires through `ApiState::fire`: budget, deferral, per-owner cleanup.
  A write inside a handler lands after the delivery's cursor, so it fires next step.
- Connect-time hooks were avoided on purpose: `Connect` (main), `Once` and `Wait` (#404) go through different `Hub`
  paths. Instead a VM *watches* an instance from the first `inst.Changed` read / `GetPropertyChangedSignal` call until
  the instance is destroyed (`PropWatch` in `api/property_signals.rs`). Firing goes through the hub, so any listener
  kind (handler, Once, Wait waiter) is served without knowing about it. A VM watching nothing never reads the journal
  (`ScriptHost::property_changes_scanned` test hook stays 0). Changes made before the watch began are skipped (each
  watch stores the cursor it started at).
- Core per-property signals (`World::property_changed`) are untouched; this adds no handler there.
- Signals are plain `LuaSignal`s keyed `(id, EventKind::Changed)` and `(id, EventKind::PropertyChanged(&'static str))`
  (name from `PropDef::name`, so `SignalKey` stays `Copy + Hash`).
- Visibility: both VMs of a combined host share one World, as for Touched etc.; each VM gets only changes of instances
  its scripts watch and only non-hidden properties. Networked (#305) clients run over their replicated world, so they
  only see replicated changes.

## Done

- Core: journal property tag, `property_changes_since`, `PropertyChange`, `ChangeCursor: PartialOrd`; test
  `property_changes_since_lists_each_real_write_in_order` in `crates/metallic_core/tests/journal.rs`.
- Script: `api/property_signals.rs` (watch set, delivery, `GetPropertyChangedSignal`), `EventKind::{Changed,
  PropertyChanged}`, `Changed` on every instance in `instance.rs::event_kind` (reading it starts the watch), host step
  call and `property_changes_scanned` hook.
- Errors: unknown or hidden name `"<name> is not a valid property name."` (positioned like other API errors);
  `Parent` and `BasePart.CFrame` say `GetPropertyChangedSignal("X") is not supported yet`.
- Tests `crates/metallic_script/tests/property_changed.rs` (11): the ten from the brief plus
  `changes_made_before_watching_do_not_fire`.
- Docs: `docs/luau-api.md` (Instance members, signal table, "Property changes"), ARCHITECTURE.md (core + script).

## Next / open

- Follow-ups worth an issue (not filed from this session): `Changed`/`GetPropertyChangedSignal` for `Parent`
  (set_parent journals without a property) and a `CFrame` alias for BaseParts.
- Journal overflow (>16384 world changes between two steps while something is watched) drops that step's property
  events with a warning.
- A watch is per instance and only ends on destroy (not on the last Disconnect): cheap, but a disconnected watcher
  still costs one hash lookup per journal entry until the instance goes.
- PR #404 (`:Wait()`/`:Once()`): dry-run merge result is in the session report; no textual overlap was designed in.

## Commands

```
cargo test -j 4 -p metallic_script --test property_changed
cargo test -j 4 -p metallic_core --test journal
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all --check
```
