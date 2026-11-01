# #14 Developer mode

## Goal

Task 14 of `docs/superpowers/plans/2026-10-30-engine-m1.md`: a developer mode switch, a performance overlay, a log
viewer, an Instance inspector, feature flags and a Report a bug zip. Developer mode is the switch that exposes the
Profiler (#23) and the script debugger (#22); it does not duplicate them.

## Done

- `devmode/mod.rs`: `DevMode::resolve(saved, channel)` and `default_for` (on for `dev`, off for `stable`).
  `build_channel()` reads `METALLIC_STUDIO_CHANNEL` at compile time; `tools/release.py` sets it from `--channel`,
  and untagged builds count as `dev`. `DevState` (enabled, overlay, log, inspector, flags) lives in
  `EditorState::dev`, so actions can toggle it.
- Settings > Developer mode (checkbox). `settings.toml` gets `developer_mode` only once it is changed there; until
  then the channel decides. The app copies the result into `EditorState::dev.enabled` every frame.
- **Seam for other tools:** an action whose id starts with `dev.` is a developer tool.
  `ActionRegistry::register` disables it while developer mode is off and keeps it out of the Play lock, and the
  Developer menu (shown only in developer mode) lists every such action in registration order. The Profiler panel
  (#416) and the debugger UI should register `dev.profiler` / `dev.debugger` toggles; nothing here needs to change.
- `dev.overlay` (F3, Developer > Performance overlay): fps, mean and max frame time, a 180-frame graph with 60 and
  30 fps lines, heap bytes (`overlay::CountingAlloc`, installed as the Studio binary's global allocator), Part and
  instance counts of the edit or play world (every 0.5 s), the viewport's draw calls (`Renderer::draw_calls`,
  new), and while playing each script VM's memory and waiting threads plus running scripts
  (`ScriptVm::stats`, `ScriptHost::stats`, `Game::script_stats`, `PlayView::script_stats`, all new). Per-subsystem
  timing stays in the Profiler.
- `dev.log`: `log_view::install()` replaces `env_logger::init()`. The console behaves as before (`RUST_LOG`), and
  every record at info or above (plus what `RUST_LOG` enables) is kept in a 2000-line ring. The window filters
  by level (segmented) and text, copies the shown lines and clears.
- `dev.inspector`: the selection's id bits, parent, child count and every property of the class chain (hidden
  ones marked), read-only.
- `dev.flags`: `FeatureFlags` (`is_on`, `set`, `remove`, `load`, `save`, `parse`) in `flags.toml` in the config
  folder (`%APPDATA%\MetallicStudio`), with a panel to toggle, add and remove flags. Names are `a-z0-9_-.`; bad
  lines are skipped. `EditorState::flags` holds them; the app loads them when a game opens.
- `dev.report_bug`: `UiRequest::ReportBug` asks egui for a window capture, waits for it for at most 2 s, then
  `BugReport::write` makes `%LOCALAPPDATA%\MetallicStudio\bug-reports\<UTC timestamp>.zip` with `version.txt`
  (version, channel, GPU from the wgpu adapter, OS), `log.txt`, `output.txt` and `screenshot.png`, and opens the
  folder. The zip is written by hand (stored entries, CRC-32), so there is no new dependency. Text entries go
  through `Redactor`: the stored GitHub session from the credential store (and each long quoted string in it),
  anything shaped like a GitHub token (`ghp_`, `gho_`, `github_pat_`...), and the home folder (replaced by `~`).
- Screenshot scenes `--scene devmode` (Play running, overlay and inspector) and `--scene devpanels` (Log and
  Feature flags).

## Tests

- `tests/devmode.rs`: the plan's five (`dev_channel_enables_devmode_by_default`,
  `stable_channel_disables_by_default`, `flags_round_trip`, `bug_report_zip_contains_logs_version_and_screenshot`
  reading the zip back through its central directory and checking every CRC, `bug_report_has_no_tokens` scanning
  the whole zip for the stored tokens), plus the saved choice, `dev.*` gating and Play exemption, a later `dev.*`
  registration joining the menu, and a kittest pass: F3 shows the overlay, the three windows draw, and turning
  developer mode off hides them.
- `devmode::bug_report` unit tests: CRC check value, UTC timestamps, token and home redaction, JSON session
  secrets, report folder.
- `metallic_script/tests/vm.rs`: `stats_count_memory_and_waiting_threads`.

## Screenshots

`docs/agents/14-shots/devmode.png` and `devpanels.png` (1440x900 captures, scaled to 1080 wide). The fps in them is
from a debug build in its first frames.

## Limits

- The heap figure counts Rust allocations only (not GPU memory or the driver).
- Draw calls do not include the post-processing chain.
- Report a bug writes nothing in screenshot mode (`can_save` false).
