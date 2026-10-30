# Issue 190: window-free Launcher core

## Goal / scope

Implement Launcher L1 Task 2 independently of updater Task 1: deep links, exact origin and navigation policy,
command-line isolation and bounded, atomic settings storage. No UI, process launching, bridge, sign-in or installer.
Claimed after verifying no blockers/fresh competing claim and merged plan PR 188.

## Implementation

- New `metallic_launcher` library only. Workspace `crates/*` membership needs no root manifest edit. Cargo.lock adds
  only the local package and edges to existing locked versions of url/serde/toml/thiserror/log/tempfile.
- Typed `PlaceId`, redacted `JoinTicket`, `SitePath` and canonical `DeepLink`. Query values decode once; protocol
  authority/actions are checked before URL normalization can erase userinfo, empty ports or dot segments.
- Exact scheme/host/effective-port policy; credentials, lookalikes, HTTP downgrades, non-default local-page ports and
  dangerous schemes never receive site/local privileges. Paths cannot replace an origin. `about:blank` can load but
  grants no capabilities. Unconfigured production placeholder stays explicit.
- Any protocol argument position disables flags and persisted developer settings (stricter than the plan's original
  first-position-only wording, as requested by root). Arguments after it are ignored; malformed links fail closed.
- Bounds precede parsing/accumulation: URI/origin/argument 2048 bytes, 64 ordinary arguments, site path 512 bytes,
  navigation 16 KiB, settings 64 KiB. Errors/warnings do not echo inputs; settings Debug redacts the developer URL.
- Settings patches validate as a whole before mutation. Unique same-directory temporary files avoid a fixed `.tmp`
  collision; write/sync/close then atomic replacement. Permission/sharing failures retry at 10/20/40 ms, preserving
  old settings and cleaning the temporary file on final failure. Saves are whole-file last-writer-wins, not edit merging.

## Tests-first / research

- Initial integration tests failed to compile on missing core types, as the Task 2 plan expected.
- A later raw-protocol normalization regression failed before strict raw-shape validation, then passed.
- Concurrent writer/reader test reproduced actual Windows PermissionDenied before the bounded persistence retry;
  closing the writer handle alone did not fix it. Concurrent saves/readers now pass, while a persistent Windows
  share lock still errors safely without changing old bytes or leaving a temp file.
- Focused `cargo test -p metallic_launcher -j 4`: 39 tests passed (13 deep-link, 13 settings, 13 policy).
- Primary sources reviewed: [url 2.5.8](https://docs.rs/url/2.5.8/url/struct.Url.html),
  [tempfile atomic persist](https://docs.rs/tempfile/3.27.0/tempfile/struct.TempPath.html#method.persist),
  [Rust filesystem rename](https://doc.rust-lang.org/std/fs/fn.rename.html). Installed tempfile Windows source confirms
  MoveFileExW replacement and retaining TempPath on failure; the closed TempPath is kept between bounded retries.
- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: 251 tests passed.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed after replacing one boolean match with
  the equivalent `matches!` spelling. Focused launcher suite rerun after that syntax cleanup: all 39 tests passed.
- `cargo deny check licenses`, `cargo fmt --all --check`, `git diff --check`: passed; existing unused MPL-2.0 license
  allowance warning only. No dependency version changes.
- Independent read-only review of URL/deep-link/argument/settings source and tests found no actionable issue.
- Guarded lane116 ownership transition cleaned local packages only; compilation confirms this worktree. Coordinated
  timed test windows with other agents. Final local logs: `metallic-190-workspace-final.log`, `metallic-190-clippy-final.log`.

## Next / open questions

Implementation, local gates and independent source review are complete. Push/open the PR and mark issue 190 for
review; CI remains pending. No merges. Later tasks build the shell and consume these APIs. No production origin or
credentials are supplied by this change.
