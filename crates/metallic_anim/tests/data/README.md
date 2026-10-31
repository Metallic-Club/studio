# Pre-migration animation fixtures

Emitted before manifest changes by main `a0bb1bfc0e84ffd637fe8d1759f4822322601cd8`
with glam 0.30.10 and the existing metallic_anim RON writer. The small generator
and baseline logs are retained in external `integration-evidence/106-shared-glam`;
the generator SHA-256 is
`ecd8eb646c74aff5e05b907463c15d5d4a33d04682bdc3558a1d4e4edb759d9e`.

The clip contains nonzero Vec3 translation and nonidentity unit-quaternion keys.
The skeleton contains a parent/child hierarchy, nonuniform scale, translation and
nonidentity rotations. New tests passed with the old dependency before migration.
They check fixed expected values and emitted field/component order, rather than
only generating and round-tripping a file with the new version. Line endings are
normalized for Git's platform checkout behavior; values and expected text remain
unchanged. This guards compatibility, not a claim that an old format bug exists.

LF-normalized fixture SHA-256:

- `clip-glam-0.30.ron`: `384d6486700e5e554c24c6bfee4ed193b49eb6a8553d4aef0a780bca9c4ae60d`
- `skeleton-glam-0.30.ron`: `d51c6d8bae58599d7b5e980e2066a7579c9d0e142e322c0d09a4d8b31ead1dd8`
