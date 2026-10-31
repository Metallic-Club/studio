# 288: Studio uses its own data folders (MetallicStudio)

Status: done on branch `studio/288-studio-folders`, ready for review.

- New `crates/metallic_studio/src/data_dirs.rs` (same pattern as the Launcher's `data_dirs`, PR #241):
  `STUDIO_FOLDER = "MetallicStudio"` (the Velopack pack id), marker file `.metallic-studio`,
  `config_dir_in(base)`, `resolve_config_dir(env)`, `create_owned`, `remove_owned`, `clean_up_in`.
- Settings now live in `%APPDATA%\MetallicStudio` (was `%APPDATA%\Metallic\Studio`); Linux/macOS
  fallbacks use `MetallicStudio` too. `METALLIC_STUDIO_CONFIG_DIR` still overrides.
- Studio had no other data/cache/log/crash paths (only recent games and recent colours, both in the
  config dir). A new config folder gets the marker; an existing one is never marked or deleted.
- `remove_owned` deletes only folders that hold the marker. Nothing calls cleanup yet (no uninstall
  hook in Studio); the helper is there for it.
- Tests: `crates/metallic_studio/tests/data_dirs.rs` (injected temp bases only).
- `tools/test_release.py`: added an assertion that the Studio pack title stays "Metallic Studio".

## Ruling: no migration
No migration from `%APPDATA%\Metallic\Studio`. Only 0.1.x dev builds used it, and reading another
app's folder is the risk being removed. Users of those builds lose their recent-games list once.
The old folder is left untouched.
