# 194 Launcher: installs and updates

Branch `studio/launcher-l1-20261030` (shared by all Launcher L1 tasks). Closes #194.

## Done
- `metallic_update`: `ReleaseAsset` and `UpdateChecker::latest_asset` (needs GitHub's `sha256:` digest, asset address must be on the API host),
  `HttpClient::get_to_writer` (UreqClient: manual redirects, `Authorization` dropped when the host changes, plain http only to this PC),
  `download::download_verified` (`.part`, size cap, SHA-256, rename; failure deletes the part file; stale files are replaced, never reused),
  `UpdateChecker::apply_after_exit(restart)`, serde for `Channel`.
- Launcher: `install.rs` (Studio install, statuses, GitHub device sign-in), `self_update.rs` (self-update, `ProcessTracker`), `settings.channel`
  (local-only), tray "Restart to update", `Spawner::spawn_and_wait`, settings page (channel, "Sign in to GitHub for team builds").
- Bridge: `installStatus`, `installStudio`, `githubSignIn`, `restartToUpdate` are real. In-flight cap now counts calls the host is running (I-3).
  `githubSignIn` events go to bundled pages only.
- Tests: `metallic_update/tests/updater.rs`, `metallic_launcher/tests/install.rs`, `tests/bridge.rs`.

## Decisions
- The Player is not installed here: it ships beside the Launcher and updates with it.
- The channel is read at start; a change applies after a restart.
- Setup.exe is run with `--silent`. Not verified against a real Velopack installer (never run on this PC).

## Next
- Task 7 (Metallic sign-in). Check `--silent` and the installed-copy flow on a clean test PC.
