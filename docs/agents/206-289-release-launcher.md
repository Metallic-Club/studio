# 206 / 289: release, sign-in and Launcher follow-ups

Branch `studio/206-289-release-launcher`. What changed, by issue item.

## #206
- Tag check wording: `docs/releasing.md` and a comment in `release.yml` say the "tag must be on `main`" step catches
  mistakes only; the tag ruleset is the required control (a tag runs the `release.yml` of the tagged commit).
- Refresh network blip: `GithubAuth::valid_token` keeps the current token when the refresh fails with a network error
  and the token has not expired yet (test `refresh_network_blip_keeps_the_still_valid_token`).
- Credential names: docs and comments now say `github.MetallicStudio` (keyring's `<account>.<service>`), and that
  `CRED_PERSIST_ENTERPRISE` roams the session (accepted).
- UI threading: package downloads already had a ten-minute timeout (`PACKAGE_TIMEOUT_MS`). New
  `GithubAuth::device_sign_in_until(ui, cancel)` (test `device_sign_in_can_be_cancelled`). `check()` is documented as
  blocking, to be called off the UI thread. The Launcher already runs all of them on workers; Studio has no UI for them
  yet.
- `deny.toml`: OFL-1.1 and Ubuntu-font-1.0 are exceptions for `epaint_default_fonts` only (`cargo deny check licenses`
  passes).

## #289
- `--hidden` starts in the tray (`show_on_start`; shows anyway without a tray icon).
- Uninstall deletes the Credential Manager entries `platform.MetallicLauncher` and `github.MetallicLauncher`
  (`forget_saved_sign_ins`; tested with in-memory stores only).
- Tray "Open Studio" appears after a Studio install (`TrayMenuState::on_event` looks at the disk again).
- The bridge's own Play limit is gone (`LaunchGate` covers every start); the GitHub page opens through the Launcher's
  https-only `platform::open_in_browser` after `github_auth::is_github_page`.
- `getInfo.channel` is the running channel (`LauncherInfo::running`).
- The pipe client also checks that the server's image is named like its own (`same_program`).
- Studio's `vpk pack` line already passes `--runtime win-x64` (pinned in `tools/test_release.py`).
- More than 250 releases behind: the check fails closed and offers the newest version as required.
- Left for the owner (need a desktop session or a published release): retake the Launcher screenshots after the Metallic
  Play rename; acceptance of close to tray and tray Quit, an update applied on quit (needs a `launcher-v*-dev` release),
  and Windows 10 without WebView2.
