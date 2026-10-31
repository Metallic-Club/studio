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
  (for example the `hit` of a `TouchEnded`). The table is emptied at the end of the step, so the values live only as
  long as a Luau reference.
- **Reading a destroyed instance.** Its last property values (`Name`, `ClassName`, `CFrame` from `Position` and
  `Orientation`, ...); `Parent` is nil; `FindFirstChild` nil; `GetChildren`/`GetDescendants` `{}`; `IsA` answers;
  child-by-name access and events error. Setting `Parent` errors with "The Parent property of X is locked: it has been
  destroyed", other writes with "cannot set P: X has been destroyed"; `Clone` and `WaitForChild` error. If its values were
  not kept (more than `DESTROYED_LIMIT` destroyed between two steps), only `Name` and `Parent` read.

## Tests

`metallic_core/tests/tree.rs`: kept only when asked, bounded, dropped when turned off. `metallic_script/tests/api.rs`:
a destroyed, never-seen limb in a `TouchEnded` handler reads `Name`, `ClassName`, properties, `Parent` nil, empty
children, and the `hit.Parent and hit.Parent:FindFirstChild("Humanoid")` guard; writing `Parent` or a property
errors; a script-held instance destroyed by the engine (and its slot reused) keeps its renamed `Name`; nothing is kept
between steps or after `stop`.

## Not done

- A destroyed `Model`'s `WorldPivot` and other values computed from children are not kept.
