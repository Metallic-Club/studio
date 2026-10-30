# Issue 88: template settings survive project creation

## Goal

Keep creator-facing settings metadata in the created project and agree on the template copy contract across code,
the M1 plan and the template authoring proposal. No settings panel or runtime attribute API is introduced.

## Implementation

- Canonical `[[setting]]` metadata lives in the copied `metallic.toml`. Renaming preserves every other TOML value,
  including ranges, help, choice lists and extension tables. Runtime `ProjectConfig` accepts this metadata.
- Legacy settings from `template.toml` migrate during creation without changing the source. Identical dual definitions
  are retained once; different definitions fail with `StudioError::Metadata` and existing rollback behavior.
- Exclude root catalog metadata, default preview files, configured template-relative preview files and root `tests/`.
  Copy tutorials, scripts and nested project assets. Preserve root `metallic.toml` even if incorrectly listed as a preview.
  Symlinks remain skipped. Preview path normalization cannot escape the template root.
- Eight regressions cover metadata preservation, legacy migration, conflict rollback for new/existing empty targets,
  deduplication, precise exclusions, mistaken preview references, a shipped Parkour project loaded by `Game::load`
  and directly supplied `TemplateInfo` without a catalog file.

## Validation

- Tests first: the initial 14-test template suite failed three new regressions (legacy loss, ambiguous settings,
  preview/test copying); eleven passed. After implementation all 15 then-existing tests passed.
- The first full run found the existing start-screen direct-`TemplateInfo` fixture did not supply a catalog; adjusted
  the metadata read to tolerate a missing catalog while retaining errors for malformed/unreadable existing files.
- Final `cargo test --workspace -j 4 -- --test-threads=1`: 220 passed, including 17 template integration tests and
  the shipped Parkour create/load regression. Focused `cargo test -p metallic_studio -j 4 -- --test-threads=1`:
  49 passed (14 library, 17 template, 18 updater).
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed; existing unused MPL-2.0 allowance warning only. No dependencies added.
- `cargo fmt --all --check` and `git diff --check`: passed. No visual change or screenshot requirement.
- Rebased/current with `origin/main` at `ebb94f3` before final gates. Logs are local temporary files
  `metallic-88-workspace-final.log` and `metallic-88-clippy-final.log`.
- Cargo uses the verified agent-owned `lane116` target. Before switching from issue 98, cleaned only the seven local
  workspace packages to avoid stale same-version artifacts; dependency artifacts were preserved. Initial test compile
  output confirmed source paths under `88-template-settings`.

## Next

Implementation and all local gates are complete. Push the issue branch, open a PR, mark issue 88 for review and
await independent review/CI. Do not merge it.

## Limits / open questions

Storage and copy behavior are implemented. Settings schema validation, knob application and the settings panel remain
planned work. No new dependencies, fake Parkour controls or UI changes are included. TOML values survive; formatting
and comments are not preserved by the pre-existing parse/serialize rename operation.
