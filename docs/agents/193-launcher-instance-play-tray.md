# 193 Launcher: single instance, metallic:// links, Player launch, tray

Branch `studio/launcher-l1-20261030` (shared by all Launcher L1 tasks). Closes #193.

## Done
- `single_instance.rs` (codec, traits), `protocol.rs` (registry logic), `launch.rs` (Player and Studio start), `tray.rs` (menu and close rules).
- `platform/windows.rs`: named pipe (first instance, no remote clients, current user only, server user checked before sending), HKCU registry,
  `SystemSpawner` (no shell, no console), tray icon (`tray-icon`), Velopack hooks (register after install and update, clean up on uninstall).
- `main.rs`: instance check before any window; second start sends the canonical link or a focus request and exits 0 (1 when sending fails, 2 for a bad link).
  `--exit-after-load` (test hook) skips the check.
- `shell.rs`: deep links (Play, StudioOpen, Open, Install), hand-over listener thread, tray events, close hides to tray when `closeToTray` is on and a tray exists.
- M-5 fixed: `AppEvent::RunScript` goes through `Bridge::guard_script` (same `location.origin` guard as replies).
- Tests: `tests/{single_instance,protocol,launch}.rs`, two in `tests/bridge.rs`; pipe, real-registry (test key) and spawn tests in `platform::windows`.
  Stand-in Player: `examples/stand_in_player.rs` (writes its arguments to `<its path>.args.txt`).
- Screenshot: `docs/agents/launcher-l1/tray-menu.png`.

## Decisions
- Pipe and registry tests are unit tests inside the program crate (`platform` is not part of the library).
- Install on a missing Player or Studio shows the site's `/downloads` page (or the settings page offline).
- `Check for updates` in the tray only logs until Task 6.

## Next
- Task 6 (install, update, channel), Task 7 (sign-in). The real `metallic-player` must accept the play link argument.
