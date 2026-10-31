# 390: destroyed instances stay readable

Issue #390. Since #370, `TouchEnded` hands scripts the destroyed limb of a character that was destroyed or replaced,
and reading any member of a destroyed instance raised an error, so `if hit.Parent and ...` failed.

## What changed

- **Core (`metallic_core::World`).** `keep_destroyed(bool)` (off by default) makes `destroy` move each destroyed node's
  class and property map into a `DestroyedInstance` (a move, no copy), kept until `take_destroyed`. At most
  `DESTROYED_LIMIT` (16384) are kept, so a world nobody drains stays bounded; turning it off drops them. Reads, writes
  and `deep_clone` do no new work (the `instructions` gate covers those).
- **Script host.** `start` turns keeping on and `stop` turns it off. When a script touches a destroyed instance, or at
  the end of each step, the host takes the world's `DestroyedInstance`s into a per-step table; each live Luau object
  of one (found through the weak wrapper cache) keeps an `Rc` of it, and so does an object made later in that step
  (for example the `hit` of a `TouchEnded`). The table is kept for one more step, then emptied, so the values live only as long as a Luau reference, plus
  one step.
- **Reading a destroyed instance.** Its last property values (`Name`, `ClassName`, `CFrame` from `Position` and
  `Orientation`, ...); `Parent` is nil; `FindFirstChild` nil; `GetChildren`/`GetDescendants` `{}`; `IsA` answers;
  child-by-name access and events error. Setting `Parent` errors with "The Parent property of X is locked: it has been
  destroyed". Other writes work as in Roblox: they change this Luau object's record only (copied on first write, so
  the other VM's object keeps the old values) and never bring the instance back. `Clone` and `WaitForChild` error. If its values were
  not kept (more than `DESTROYED_LIMIT` destroyed between two steps), only `Name` and `Parent` read.

## Review fixes (FIX-FIRST on 1fcdd049)

- Writes other than `Parent` are allowed and go to the record (Roblox behaviour), instead of being refused.
- The step's table is kept one more step. A character destroyed by a script during a step reached scripts in the
  next step's `TouchEnded` after the table was cleared, so `hit.Name` read "" and other reads errored.

## Tests

`metallic_core/tests/tree.rs`: kept only when asked, bounded, dropped when turned off. `metallic_script/tests/api.rs`:
a destroyed, never-seen limb in a `TouchEnded` handler reads `Name`, `ClassName`, properties, `Parent` nil, empty
children, and the `hit.Parent and hit.Parent:FindFirstChild("Humanoid")` guard; writing `Parent` or a property
errors; a script-held instance destroyed by the engine (and its slot reused) keeps its renamed `Name`; nothing is kept
between steps or after `stop`; a character destroyed by a script mid-step is readable in the next step's
`TouchEnded`, and the table is gone a step later; writes to a destroyed instance stay on its record (Size clamped,
CFrame stored as Position and Orientation) and it never returns to the tree.

Merged with #420 (lossless tag events). `GetInstanceRemovedSignal` handlers can run steps after the destroy when a
step runs out of time, so the host now keeps each step's records in a batch until one more step has passed and every
VM's tag cursor is past the batch (capped at `MAX_DESTROYED_KEPT` = 65,536 records). The world keeps tagged
instances past `DESTROYED_LIMIT`, since the tag log names each one. `GetAttribute`, `GetAttributes`, `HasTag` and
`GetTags` on a destroyed instance return what it had; `SetAttribute`, `AddTag` and `RemoveTag` still error (documented
deviation). Tests: `tests/tags.rs` destroys 20,000 tagged parts in one step and every deferred removal handler reads
`Name`, `HasTag` and `GetAttribute`; the records are gone once delivered. `tests/api.rs`: attribute and tag reads on a
destroyed instance.

## Not done

- A destroyed `Model`'s `WorldPivot` and other values computed from children are not kept.
