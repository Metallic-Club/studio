# #202 Exact-release update source

Status: implemented and locally tested; independent code review passed; PR/CI pending.
Branch: `studio/202-tagged-update-source`, based on `ebb94f3`.

## Goal and cause

Studio's check can discover a stable version among up to 250 releases, while Velopack's built-in GitHub source only
lists the ten newest. Enough dev releases therefore hide a stable package during download. Pin the download source
to the already selected version/channel rather than running a second, narrower discovery pass.

## Implemented

- Private `updater/tagged.rs` implements Velopack `UpdateSource`; Studio's manager uses it for the requested update.
- Fetches `/repos/{repo}/releases/tags/vX.Y.Z-channel`, validates the published tag, and caches its asset metadata.
- Fetches the matching channel feed and offers only the selected version. Requires a full package and resolves every
  advertised full package within the selected release. Only full packages are offered: this source cannot prove the
  complete delta chain from an arbitrary skipped version. Velopack verifies the full transfer's checksum.
- Authenticated asset URLs are constructed from numeric IDs at the configured API origin; remote metadata URLs and
  feed filenames cannot redirect the initial token-bearing request to an arbitrary host.
- Uses existing injectable metadata HTTP client and Velopack streaming package transfer/checksum verification. Each
  transfer has a ten-minute overall timeout. No new HTTP trait or public Studio interface was added.
- Enables Velopack's `public-utils` feature to implement its public source trait with `bundle::Manifest` and reuse
  its download helper; version and dependency graph are otherwise unchanged.

Primary references: [GitHub release-by-tag API](https://docs.github.com/en/rest/releases/releases#get-a-release-by-tag-name),
[binary asset API](https://docs.github.com/en/rest/releases/assets#get-a-release-asset), and the installed pinned
Velopack 1.3.39-g8d2844f `sources/github.rs`, `sources/mod.rs`, `download.rs`, `manager.rs` and feature definitions.
The pinned built-in source confirms the ten-entry listing; no external release or credential was accessed.

## Verification so far

- Initial seven targeted unit tests passed, including a loopback HTTP route with fifteen newer dev releases, stable discovery,
  exact tag/feed lookup, cached package lookup, binary NUL/non-UTF-8 bytes, authenticated requests and progress to 100%.
- Negative tests reject wrong channel/tag/version, draft releases, malformed/missing/ambiguous assets or feeds, and
  hide response/transport secrets in errors. Foreign-version package rejection happens before network/file writes.
- `cargo fmt` / `git diff --check` passed.
- Used reviewed #209 guard externally with root-owned `lane122`; it cleaned local workspace packages on ownership
  transition and compilation paths identify this worktree. Final workspace/Clippy/license checks pending.
- Independent review caught unavailable optional deltas blocking a valid full package and the deeper incomplete-chain
  risk after filtering history. The source now offers full packages only; a regression covers absent and attached deltas.
- The initial full workspace gate exposed a Windows loopback fixture bug: accepted sockets inherited the nonblocking
  listener mode, so an early WouldBlock was mistaken for EOF. The fixture explicitly makes accepted sockets blocking
  before applying read/write timeouts. Production transport behavior was unchanged. All eight final targeted tests
  then passed in the same workspace feature configuration that exposed the failure.
- Final `cargo test --workspace -j 4 -- --test-threads=1`: 220 passed, including all eight updater-source regressions,
  the complete Parkour route, player smoke, hostile-script and existing updater/sign-in tests.
- Final `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo deny check licenses`, `cargo fmt --check`
  and `git diff --check`: passed. No new dependencies or lockfile changes.
- Independent read-only review rechecked full-package checksum routing, redirect authorization stripping in the
  pinned HTTP implementation, tag/channel/version validation, error sanitization and local fixture bounds; no
  additional actionable findings remained. No installed apply/restart claim is made.

## Next and limits

Commit/push/PR and CI. Do not merge. Real installed-copy apply/restart and the private
GitHub App/release round trip still require the existing owner acceptance process. This task does not address #203's
required-update pagination, #204's refresh-token race, #205's workflow privileges or #206's UI threading/cancellation.
