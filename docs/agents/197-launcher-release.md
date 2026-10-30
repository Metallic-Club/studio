# 197 Launcher: packaging, release, acceptance

Branch `studio/launcher-l1-20261030`. Task 9 of the Launcher L1 plan.

## Done (code and CI, seen working locally unless noted)
- `tools/release.py --app studio|launcher` (default `studio`). Studio's `--dry-run` output was recorded before and after the
  change and is byte-identical (dev, and stable with `--required`); `tools/test_release.py` pins Studio's `vpk pack` line.
  Launcher dry-run prints `vpk pack --packId MetallicLauncher --mainExe Metallic.exe --framework webview2 --channel launcher-dev`.
- `.github/workflows/release.yml`: tag `launcher-v*` runs the Launcher release; `v*` stays Studio.
- `.github/workflows/ci.yml`: job `launcher-linux-check` (Ubuntu). **Not run locally; it runs when the branch is pushed.**
- Hardening H1: if the WebView2 frame-navigation handler cannot be registered, the Launcher shows the bundled offline page and
  refuses to load remote pages (`remote_content_allowed`, tested; the Windows registration failure itself was not provoked).
- Hardening H2: `PlayLimiter` (burst 3, then 1 per 2 s) on Play links handled by `handle_deep_link` (tested).
- Docs: `docs/releasing.md`, `ARCHITECTURE.md`, `docs/STATUS.md`.

## Step 3 findings (lead's real install test)
- The old `%LOCALAPPDATA%\Metallic\` belongs to another app called "Metallic". The Launcher now uses `MetallicLauncher\data` and
  `%APPDATA%\MetallicLauncher`, writes a `.metallic-launcher` marker when it creates them, and uninstall deletes only marked
  folders. Keyring service is `MetallicLauncher`. No migration (nothing shipped).
- Shortcut and Apps-list name collides with any other app titled Metallic; the product name is an owner decision. (`--packTitle Metallic` kept for now.)
- `vpk pack` for the Launcher passes `--runtime win-x64` (otherwise vpk warns and uses x86 helpers). Follow-up: Studio's line likely needs the same; left unchanged to keep its output identical.

## Acceptance checklist (Step 3): pending: lead/owner round trip
Needs a release build, a pushed tag and a Windows 11 test PC. Record each with a screenshot under `docs/agents/launcher-l1/`.
- [ ] Install `MetallicLauncher-launcher-dev-Setup.exe` on Windows 11 without admin.
- [ ] `HKCU\Software\Classes\metallic` exists.
- [ ] A browser link starts the Launcher and the Player (real Player if it accepts the URI by then, else the stub).
- [ ] A second link while running is handed over to the running Launcher.
- [ ] Close goes to the tray; tray Quit quits.
- [ ] Publish the next dev version; it is applied on quit.
- [ ] Uninstall removes the protocol key and the data folders.
- [ ] Windows 10 VM without WebView2: Setup installs it, or the message box appears.
- [ ] Studio install from the Launcher: `--silent` Setup run (no dialog, exit 0, Studio not started, `current\sq.version` exists).
- [ ] Studio and the Launcher each ignore the other's releases (code: `version_from_tag` filters by tag prefix, tests `launcher_feed_ignores_studio_tags` and `studio_feed_ignores_launcher_tags`; confirm live after publishing both).
