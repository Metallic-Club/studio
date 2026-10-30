# #205 Release build/publish credential isolation

Status: implementation and isolated helper tests complete; full gates pending.
Branch: `studio/205-release-credential-isolation`, from main `ebb94f3`.

## Goal

Keep publishing permission off the runner that executes Cargo and dependency build
scripts. Preserve the installed executable, Velopack asset names and delta flow.
No actual release, signing, installation or publication is authorized here.

## Done

- Rechecked claims/dependencies and claimed #205. Separate fresh-main worktree.
- Split the release workflow into a read-only build job and a fresh write-permission
  packaging/upload job, both checkout-pinned to the event SHA with persisted Git
  credentials disabled. Removed release cache and post-checkout authenticated fetch.
- Added explicit CLI phases: build-only exits before token or packaging handling;
  publish-only requires an existing executable and never invokes Cargo. Cargo also
  receives an environment without GITHUB_TOKEN/GH_TOKEN in the manual combined flow.
- Added immutable same-run artifact-ID handoff of a single unarchived binary, with
  explicit fail-closed digest checking and decompression disabled. Artifact files
  never supply scripts, notes or icons to the publisher.
- Inspected Velopack pinned commit `8d2844f`: Windows pack calls CompatUtil to parse
  PE/assembly metadata; it does not launch the native Studio input through that path.
- Verified current upstream pins via official release/tag APIs: upload-artifact
  v7.0.2 `cf430e030ddbb5b0abf93d22962f4752f3646cd9`; download-artifact v8.0.2
  `9000827ccba6bdab643e8b6fd33ac0654aef8333`; setup-python v7.1.0
  `6d4fd22b20465ccc0f7354476588e8df7c9ca0e0`. Explicit Python 3.13 supplies tomllib.
- Filed linked #236 for the pre-existing package/binary-name mismatch. Separating
  package `metallic_studio` from binary `metallic-studio` is necessary for the real
  handoff; installed `Metallic Studio.exe`, pack ID and release asset flags stay unchanged.
- Staging validates its regular input before deleting generated files, verifies
  the resolved recursive-delete target, and refuses input inside its own stage.
- Added 18 standard-library tests, including mocked phase execution, no-token Cargo
  environment, real temporary staging, unchanged delta/pack/upload flow, dry runs,
  manifest alignment, and fixed workflow credential/artifact contracts.
- Added a separate read-only PR workflow for those helper tests; no shared CI file edits.
- Updated releasing docs with the precise credential boundary and limits. Tag
  authority follow-up remains #206; no unrelated updater/sign-in changes.

## Validation so far

- `python -I tools/test_release.py`: 18 passed. One initial fixture error came from
  unpatched icon paths; correcting the isolated fixture resolved it.
- Actual build-only and publish-only `--dry-run` CLI commands passed, running no
  subprocesses and creating no build/stage output.
- `actionlint 1.7.12 -shellcheck= -pyflakes=` on both release workflows: passed.
  Downloaded the official Windows tool into a task temp directory and verified
  SHA-256 `6e7241b51e6817ea6a047693d8e6fed13b31819c9a0dd6c5a726e1592d22f6e9`.
- `git diff --check`: passed. No Rust, Cargo or app dependency changes.

## Next

- Independent review and full repository gates when the coordinator's Rust build
  window is clear; use owned lane with forced local-package cleanup for provenance.
- Rebase latest main, commit/push, open and attach one PR for #205/#236, inspect CI.
  Do not publish a release or merge the PR.

## Limits

This isolates publishing credentials from Cargo execution. It does not certify
dependency-built application contents or add Authenticode signing. End-to-end
Velopack publishing remains an owner round trip; all publisher subprocesses were
stubbed in tests and real commands were dry runs only.
