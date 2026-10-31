# #11 Play and Stop in Studio

## Goal

F5 / Play runs the game on a copy of the edit world inside Studio; Stop (Shift+F5 or Esc) drops the session and the
edit world, its undo history and the selection are exactly as before. Plan: "Task 11: Play and Stop in Studio" in
`docs/superpowers/plans/2026-10-30-engine-m1.md`.

## Base

Branch `studio/11-play-stop` from `origin/main` (`8bc3cd73`), with open PR #358 (#356, `LocalPlaySession`) merged in
by a normal merge, then `origin/main` merged in (#337 building core, #366 exposure, #367 status bar). This PR lands
#358 too (Closes #356). Small changes to #358's files here:
the Output tag is now `play` (`PLAY_TAG`), and the session owns an `InputRuntime` and implements `PlayView`.

## Done

- `play::LocalPlayMode` (`play.start`, label "Play"), registered by `EditorState::new`. Combined client and server in
  one process, the same `Game` as the Player (`Game::from_world` on `World::deep_clone`).
- Seam `play::PlayView` (`PlaySession::view` / `view_mut`, default `None`): input clock, `push_input(RawEvent)`,
  `release_input`, `render`, `paint_gui`, `inspect_world`. Team Test (A12) runs in other processes and keeps `None`.
- `play::input::PlayInput`: egui keys (physical key), Shift/Ctrl/Alt from modifier state, mouse buttons, mouse motion,
  pointer position and wheel become `RawEvent`s. Keyboard goes to the game unless a text field has focus; mouse while
  the pointer is over the viewport or a button pressed there is held. Losing the keyboard releases everything.
- Frame order while playing (`editor_ui.rs`): Studio keys first (keymap contexts Global + new `Play`: Esc stops,
  Shift+F5 stops), then game input, then `PlayController::update`, then the viewport draws the game every frame with
  the game camera, avatar and ScreenGuis, framed by a `success` line. After Stop the renderer drops the avatar and
  redraws the edit world from the untouched editor camera.
- Edit lock: `apply`, `record`, undo, redo refuse while playing (status note "Stop the game to edit"); every action
  outside `actions::PLAY_SAFE` (Play, Stop, Save, Settings) is disabled, accessories too, `edit.select_all` included;
  Insert menu off; gizmos, body drags and #337's editable drag readout off; Properties shown disabled under a banner;
  script editor read-only; Explorer shows the play world read-only (`explorer::PlayTree`, virtualised with
  `ScrollArea::show_rows`).
- Keys while playing: a disabled action does not consume its key (`Keymap::pressed_where`), so the game sees Ctrl+A/D/
  S/Z/Y/G, Delete and F2; Studio keeps F5, Shift+F5, Esc and Ctrl+S. Tab, Space and Enter are taken in the app's
  `raw_input_hook` and go to the game, so egui cannot move focus or press a focused button. With the unsaved-changes
  prompt open, Esc closes the prompt and game input is released.
- Exposure adaptation resets (`Renderer::reset_exposure`) on the first play frame and after Stop.
- No per-frame allocation in the play path: input buffers are reused, key contexts are an array, `PlayStatus` text is `Cow<'static, str>`.
- Play is requested with `UiRequest::StartPlay(id)`: open script tabs are committed, a Properties gesture is recorded
  and a gizmo drag cancelled before the copy is taken; then the Scene tab shows and widget focus is dropped.
- Playing state: #367's status bar shows the session status ("Playing", `theme::success`), the viewport has a
  `success` frame, and Explorer and Properties carry `success-bg` banners (`widgets::Plate`). No raw colours. The
  earlier toolbar chip and status strip are gone: the toolbar has no room left after #337, and #367 covers it.
- Failure: a `Game::start` error goes to Output ("Could not start: ...") and Studio stays in edit mode, with a status
  note. A script that errors while loading does not fail the start (engine rule: other scripts keep running); its
  error goes to Output with the `play` tag.
- `--screenshot <png> --scene playing`: the Parkour editor with Play running.

## Tests

`crates/metallic_studio/tests/play.rs`: `play_does_not_change_edit_world`,
`stop_with_pending_delay_fires_nothing_after_stop`, `play_twice_connects_handlers_once`,
`edit_while_playing_is_refused`, `error_at_start_keeps_edit_mode`, `script_errors_go_to_output_with_the_play_tag`,
`ten_play_stop_cycles_leak_nothing` (edit and play instance counts, Position listeners and handler prints stay flat),
`f5_plays_keys_reach_the_game_and_escape_stops` (kittest: F5, W walks the character and does not pick the Move tool,
Esc stops), `tab_space_and_enter_go_to_the_game_not_to_a_focused_button` (Stop focused; Tab keeps focus, Enter does
not press it, Space jumps), `escape_answers_the_close_prompt_and_play_goes_on`. Unit tests: `play::input` key/wheel mapping, `actions::editing_actions_are_off_while_playing`,
`keymap::keys_of_actions_not_owned_stay_in_the_event_list`.

## Commands

```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_studio -p metallic_runtime -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo run -p metallic_studio -- --screenshot shot.png --scene playing --size 1440x900
```

## Open / next

- Manual Play/Stop in the Parkour template with a real window, to the finish and back (Task 12 acceptance).
- The play world is copied twice (`clean_world` then `deep_clone` in the session); fine for M1 sizes.
- Properties shows the edit values (disabled) while playing, not the live play values.
- Mouse capture (hidden cursor during a right-drag turn) is not done; the cursor stays visible.
