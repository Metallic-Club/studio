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

## Step 3 round trip (lead, Windows 11, release builds)
Done:
- Release built at 5b152ff7, then again at 1f6e444f. `vpk 1.3.39 pack --runtime win-x64` gives `MetallicLauncher-launcher-dev-Setup.exe`.
- `Setup.exe --silent` installs without admin into `%LOCALAPPDATA%\MetallicLauncher`; the install hook registered `HKCU\Software\Classes\metallic`.
- A `metallic://play` link starts the Launcher, which shows the "isn't set up yet" page (no site address configured). No Player started: it does not accept URIs yet (M2).
- A second link while the Launcher runs is handed over: still one process.
- The 1f6e444f build creates `%LOCALAPPDATA%\MetallicLauncher\data\{WebView2, .metallic-launcher}`.
- `Update.exe --uninstall --silent` exits 0 and removes the install folder, the protocol key and the Apps entry. A hash snapshot shows another app's `%LOCALAPPDATA%\Metallic` folder unchanged.

Findings: the old data-folder collision (fixed in 1f6e444f); Velopack's shortcuts overwrote a same-named `Metallic.lnk` from another app (restored by hand); `--runtime` defaulted to x86.

Not done yet: close to tray and tray Quit; update applied on quit (needs a published dev release); Windows 10 without WebView2.

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
