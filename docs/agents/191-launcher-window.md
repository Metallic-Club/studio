# 191 Launcher window, web view and offline screen

Branch `studio/launcher-l1-20261030` (shared by all Launcher L1 tasks). Closes #191.

## Done
- `Metallic.exe` (`crates/metallic_launcher/src/main.rs`): Velopack hook, logging, settings, `LauncherConfig`,
  WebView2 check (missing: message box, exit 1), then `shell::run`.
- Window `Metallic` 1280x800 (min 960x600), icon from `branding/`, hidden until the first page has loaded
  (or 2.5 s), graphite web view background; WebView2 data in `%LOCALAPPDATA%\Metallic\Launcher\WebView2`.
- Bundled pages in `assets/pages/` (offline, demo, settings placeholder), served by the `metallic-app` protocol with a strict CSP.
- Navigation, pop-ups, downloads and permissions go through `UrlPolicy` or are refused (see the manual check below).
- A failed main-frame load shows the offline page (`LoadFailed`); while offline the site is re-checked after 10 s, doubling to 60 s.
- Tests: `tests/local_pages.rs` (13), ignored `tests/shell_smoke.rs`.
- Screenshots: `docs/agents/launcher-l1/` (`offline-page.png`, `window-offline.png`, `window-demo.png`).

## Manual security check (demo page, by clicking in the real window)
- Sample Play link: handed on as a deep link (log only for now). New window to an https address: opened in the system browser.
- Download link: refused (log). Location request: refused (page says so). Local file link and `javascript:` link: nothing happens, the window stays on the page.

## Decisions
- `/healthz` on the site must answer 2xx; until the site has it, the Launcher would show the offline page. Tell the plateform issue.
- The settings page is a placeholder until Task 4.
- The Retry button posts `local:retry`; the shell accepts it only from a bundled page.

## Next
- Task 4 (bridge, real settings page), Task 5 (single instance, `metallic://` handling, tray, `--hidden`).
