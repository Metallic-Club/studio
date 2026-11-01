# 192 Launcher bridge (JS to Rust)

Branch `studio/launcher-l1-20261030` (shared by all Launcher L1 tasks). Closes #192.

## Done
- `src/bridge.rs`: `Bridge::handle/deliver/event_script/init_script`, `BridgeCommand` (typed, unknown commands and fields refused,
  `Debug` prints the name only), `allowed(origin, cmd)`, `BridgeHost`, `BridgeEvent`, `BridgeErrorCode` (fixed words, no text).
- `assets/bridge.js`: `window.metallic` (frozen, non-writable, non-configurable), main frame only, ids start at a random point.
- `shell.rs`: `ShellHost`, injection with `with_initialization_script_for_main_only`, messages and answers through the event loop.
  A message is dropped unless its page is the page on screen (stops frames and stale pages); an answer is delivered only if the page
  on screen has the same origin as the asker.
- `settings.html` is the real page (close to tray, developer mode, development address); offline Retry now calls `retrySite`.
- `restartToUpdate` (site ruling): no arguments; until the update step it answers `failed` and logs a line.
- Tests: `tests/bridge.rs` (24), ignored `shell_smoke` `dev_page_bridge_get_info` and `other_origin_page_has_no_bridge`.
- Screenshots: `docs/agents/launcher-l1/bridge-settings-page.png`, `bridge-demo-page.png`.

## Decisions
- Site pages see only `closeToTray` from `getSettings`/`setSettings`; bundled pages see developer mode and the development address.
- `play`, install, Studio and sign-in answer `notAvailable` in the shell until Tasks 5-7 fill them.
- `restartToUpdate` is for site pages only for now; Task 6 may allow the settings page too.

## Next
- Task 5 (play, single instance, protocol, tray), Task 6 (install, update, channel), Task 7 (sign-in).
