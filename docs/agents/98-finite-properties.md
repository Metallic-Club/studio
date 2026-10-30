# Finite property values: issue 98

Status: implemented; focused core tests passed; full workspace gates pending.

## Goal

Reject NaN and either infinity in numeric property values before World state, journal revisions, or property
signals change. Preserve invalid-instance, unknown-property, and type-mismatch precedence and existing enum rules.
Do not introduce numeric range clamps or reject finite extremes.

## Implementation

- An internal exhaustive Value check covers Number, all three Vector3 and Color3 components, and all four UDim2
  components. Nonnumeric variants are unchanged. The check allocates nothing on accepted writes.
- World::set applies it after instance/property/type validation and before any mutation, using the existing typed
  CoreError::InvalidValue. Public method/crate/error docs and architecture describe the behavior.
- Regression tests cover all 33 type/component combinations of NaN and positive/negative infinity; rejected writes
  preserve the previous property, cursor, empty journal delta, and connected signal count. The repeated NaN bug has
  its own regression.
- Additional tests cover error precedence, finite maximum/minimum/subnormal/zero values without clamps, and
  unchanged string, boolean, reference, and material-enum behavior.

## Verification log

- Started from main at `88f5512`; issue was ready and unclaimed before claiming.
- Verified the own lane116 target directory and cleaned only the seven Metallic workspace packages before switching
  worktrees, preserving third-party artifacts. Compiler output confirmed this worktree's metallic_core source path.
- Tests first: `cargo test -p metallic_core --test finite_values -j 4` failed five rejection regressions while the three
  precedence/finite-extreme/nonnumeric compatibility regressions passed, reproducing the reported bug.
- After implementation, `cargo test -p metallic_core -j 4`: all 31 core tests passed, including the eight new
  regressions and all journal, scene-file, and tree tests.
- Full workspace gates to finish before opening the PR.

## Remaining scope

No constructor math policy or new numeric ranges are introduced. Large finite components remain accepted under
the existing property contract; downstream conversion/range policies are separate work. No user-facing visual change.
