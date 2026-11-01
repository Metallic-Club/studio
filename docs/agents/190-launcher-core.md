# 190 Launcher core: deep links, site origin, URL policy, settings

Branch `studio/launcher-l1-20261030` (shared by all Launcher L1 tasks). Closes #190.

## Done
- New crate `crates/metallic_launcher` (library only, no window crates): `error.rs`, `deep_link.rs`,
  `site.rs`, `url_policy.rs`, `settings.rs`.
- `config/launcher_site_origin.txt` holds the placeholder `REPLACE_WITH_SITE_ORIGIN`; until the owner
  sets the real site address, `embedded_site_origin()` is `NotConfigured` and no address counts as the site.
- Tests: `tests/deep_link.rs`, `tests/url_policy.rs`, `tests/settings.rs` (the plan's tests plus extras).

## Decisions
- `SiteOrigin` keeps a parsed base `Url` next to scheme/host/port so `url(path)` needs no `unwrap`.
- A first argument starting with `metallic:` ignores every other argument and also ignores the saved
  developer mode, so a link can never turn on developer mode.
- `new_window("about:blank")` is blocked; `navigation("about:blank")` is allowed.
- Unknown start-up flags are ignored with a log line that does not repeat them.

## Next
- Task 3 adds the window (`main.rs`, `shell.rs`, `platform/`).
