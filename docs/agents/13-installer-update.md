# Task 13: installer, GitHub Releases, auto-update

Goal, done, next. Branch `studio/13-installer-update`. Full report: `.superpowers/sdd/2026-10-30-engine-m1/task-13-report.md`
in the main checkout.

Done (code, tested with mocks): `updater.rs`, `github_auth.rs`, `lib.rs` (declares only these two modules; merge with
Task 9's `lib.rs`), `tools/release.py`, `release.yml`, `docs/releasing.md`, `config/github_oauth_client_id.txt` (placeholder).
Next (owner round trip): register the OAuth app, paste client ID, publish `0.1.0-dev`, install, publish `0.1.1-dev`, update.
Wiring left for the lead: `VelopackApp::build().run()` first in `main`, check on start and every 6 h, sign-in UI.
Commands: `cargo test -p metallic_studio --test updater`; `python tools/release.py --channel dev --version 0.1.1 --dry-run`.
Open: webpki-roots licence exception in `deny.toml`; vpk flags (`--tag`, setup file name) unverified until a real run.
