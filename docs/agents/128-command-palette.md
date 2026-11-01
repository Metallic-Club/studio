# #128 Command palette (Ctrl K) and Insert at cursor (Shift A)

## Goal

Research doc section 5.4: Ctrl+K finds and runs everything (objects, assets, commands with shortcuts, guides, and a
last row that asks Litha); Shift+A over the viewport opens Insert at the cursor.

## Base

Branch `studio/128-command-palette` from `origin/main`. Builds on the action registry (`actions.rs`); #380 adds
more actions through the same `ActionRegistry::register`, and the palette lists whatever is registered.

## Done

- `palette.rs`: `CommandPalette` with modes `Search` and `Insert`, results computed only when the query changes.
  - In this level: instances by name or class, one row per name and class with path and count; Enter selects all.
  - Assets: files in the game folder (hidden files and `target` skipped, at most 5000), type and size; Enter copies
    the relative path (no asset browser yet).
  - Commands: every registered action with its keymap shortcut, disabled ones dim and not runnable, plus
    "Insert <class>". Recently run commands come first for an empty query.
  - Learn: guides given with `set_guides` (none bundled yet).
  - Ask Litha about "...": always last, Ctrl+Enter from anywhere. Until Litha exists (#20, M7) the question waits in
    `EditorUi::take_litha_question` and the status bar says so.
  - Prefixes `>` commands, `@` objects, `#` assets. Up/Down, Tab (next group), Enter, Esc, click outside closes.
  - Ranking (`fuzzy_score`): text starts with the query, a word starts with it, initials ("fs" for Frame selection),
    inside a word, then scattered letters.
  - Review fixes: objects parked for undo are skipped; `Matcher` scores in one pass without allocating, buffers
    are reused, at most 2000 distinct object names are scored and only kept rows are sorted; the file index is
    built on a background thread and reused for 10 s; the palette draws last (`Order::Tooltip`). 10k instances
    plus 5000 paths: worst keystroke 12 ms in a debug build (the test allows 40 ms in debug, 2 ms optimised).
  - `open_command_palette_with` refuses while playing, while the viewport is busy and while
    `EditorState::gesture_active()` (#380) is true.
- Keys: `palette.open` Ctrl+K, `palette.commands` Ctrl+Shift+P (opens with `>`), `insert.at_cursor` Shift+A
  (viewport). Grid snap moved from Ctrl+K to G (viewport).
- Insert at cursor: the list opens at the pointer; the item lands at `EditorState::insert_point`, the surface
  point under the cursor when the list opened, under the active container like the Insert menu.
- Entry points for the S6 title bar's search field: `EditorUi::open_command_palette()` and
  `open_command_palette_with(text)`. Not available while playing (palette actions are locked like other editing).
- Drawn with theme tokens (`bg-raised`, `line-strong`, `select-weak`, `text-*`, `R_LG`, `SHADOW_POP`) and
  `widgets` (`truncated`, new `button::keycap_at`).
- `--scene palette` screenshot: `docs/agents/128-shots/palette.png`.

## Tests

Unit (`palette::tests`): ranking, group order and counted object rows, prefixes and shortcuts, recent commands and
Tab, Enter and disabled rows, Insert at the cursor point. `tests/palette.rs` (kittest, real editor): Ctrl+K "coin"
Enter selects all three coins; Ctrl+Shift+P "duplicate" Enter runs one undo step and is remembered; Esc closes and
keeps the selection; Shift+A over the floor inserts a Part at the cursor point; Ctrl+Enter keeps the Litha question.

## Open / next

- No asset browser: an asset row copies its path. No guides index yet for Learn.
- Tab jumps forward only; Shift+Tab is not bound.
- The S6 title bar should call `open_command_palette()` from its search field.
