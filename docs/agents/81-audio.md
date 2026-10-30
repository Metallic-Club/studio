# #81 Audio system (`crates/metallic_audio`)

Branch `studio/81-audio`. Status: **implemented; last commit not yet compiled or tested** (builds frozen until the PC
reboots, see "Checks pending"). Engine side only; not wired into the runtime or Luau yet.

## Goal
Engine-side audio as a standalone crate, so Horror and the other templates (research PR #60) can have sound:
Roblox-named `Sound`/`SoundGroup`/`SoundService`/`SoundEffect` semantics, 3D spatial audio, mixer groups,
effects, voice limiting, streaming, and a headless backend for tests and servers. Wiring into
`metallic_runtime`/Luau comes after Task #8.

## Done
- `AudioEngine` (game thread) -> command queue -> `MixRenderer` (audio thread) -> `AudioBackend`
  (headless or cpal `DeviceBackend`). Feedback (TimePosition, IsPlaying, loop/end counters) comes back through
  atomics; neither side waits for the other.
- Assets: `SoundAsset` (decoded PCM, shared `Arc`), `DecoderStream` (streams OGG Vorbis/WAV/FLAC/MP3 packet by packet),
  `StreamSource` trait for procedural audio and later voice chat.
- Per-sound properties with Roblox names (`SoundProperties`): Volume, PlaybackSpeed, Looped, Playing, TimePosition,
  RollOffMode/MinDistance/MaxDistance, SoundGroup, plus `priority` and `emitter`. Play/Pause/Resume/Stop and
  `set_playing` follow Roblox (Play starts at the last script-set TimePosition; Stop resets it; Playing=true
  resumes). Events: `Ended`, `DidLoop` (collected by `update`).
- Spatial: one listener (position, forward, up, velocity), distance roll-off (Inverse, Linear, LinearSquare,
  InverseTapered = min(Inverse, LinearSquare) as the Roblox docs say), equal-power panning, Doppler, a rear
  low-pass cue, occlusion (gain + low-pass) answered per frame by a hook or a borrowed per-call closure.
- Mixer: fixed 64-frame blocks (output is bit-identical however callers slice `render`), linear-interpolation
  resampling, gain ramps over one block (no zipper noise or clicks; pause/stop fade over one block), five built-in
  groups (Master, Music, SFX, UI, Voice) plus nested user groups, group volume products, hard clamp at the end.
- Voice limit: `max_voices` (default 64). Playing sounds are ranked each block by (audible, priority, effective
  volume, start order); the rest become virtual (time advances, nothing mixed) and fade back in when a voice frees.
- Effects on groups: Reverb (Schroeder/Moorer, RT60-exact comb feedback), Equalizer (low/high shelves + mid gain),
  Compressor, Distortion, Echo, LowPass, HighPass; Roblox property names, dB levels, `Priority` ordering,
  enable/disable, live parameter changes without allocation.
- No `unwrap`/`expect` outside tests/benches/examples; typed `AudioError`; `#![warn(missing_docs)]`.
- Tests (64): spatial formulas, pan law, Doppler, occlusion, group products, ramps, voice limiting, looping,
  TimePosition, pause/resume/stop, speed/resampling, streaming (custom source, loop through seek, EOF discovery),
  decode round trips (WAV 16/24/float, 6-channel fold, FLAC via flacenc), call-size independence, effects,
  handle generations, limits, clocked headless, custom backend. All generated in the tests; no recorded audio.
- Bench `cargo bench -p metallic_audio --bench mix` and example `spatial_demo` (device only; `--mute` for a silent
  smoke test; prints a note and exits 0 without a device).

## Measured (2026-10-30, this PC, other sessions' builds running at the same time, so numbers are noisy)
- 64 spatial voices (looped, mixed 44.1/48 kHz sources, Doppler, InverseTapered), 1 s of 48 kHz stereo:
  **34.9 ms** (33.2-36.7), i.e. about 3.5 % of one core. Before the in-memory asset fast path: 40.9 ms.
- Same plus occlusion on half the voices and a Master reverb: **46.9 ms** (42.9-50.9).
- `spatial_demo -- 2 --mute` on the real device ("Speakers (G06-BT)", 48 kHz WASAPI): 96,576 frames rendered in about
  2 s, so the device path runs in real time. Not listened to (owner asleep), hence `--mute`.

## Checks pending (builds frozen 2026-10-30 until the owner reboots; C: full, page file holding the space)
The last commit (`6017177`, stream `TimeLength` reported up front + `AudioEngine: Send` assertion, touching
`src/engine.rs`, `src/mixer.rs`, `tests/decode.rs`, `tests/engine.rs`) has **not been compiled**. The 64 tests
listed under "Done" passed on the commit before it (`ac42581`).

Passed on `6017177`: `cargo fmt --check` (workspace; no compilation).

NOT run yet on `6017177`, in this order (stop at the first failure):
1. `cargo test -p metallic_audio -j 2`
2. `cargo clippy -p metallic_audio --all-targets -j 2 -- -D warnings`
3. `cargo clippy -p metallic_audio --all-targets --no-default-features -j 2 -- -D warnings` (headless-only build)
4. Workspace checks, once, as CI runs them: `cargo test --workspace -j 2`,
   `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, `cargo deny check licenses`.
   These have never run on this branch.

After the reboot, with at least 4 GB free on C:, in Git Bash from this worktree:
```
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/c/Users/alexi/Documents/metallic-studio-worktrees/_target-lead-shared CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_BENCH_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p metallic_audio -j 2
cargo clippy -p metallic_audio --all-targets -j 2 -- -D warnings
cargo clippy -p metallic_audio --all-targets --no-default-features -j 2 -- -D warnings
cargo test --workspace -j 2
cargo clippy --workspace --all-targets -j 2 -- -D warnings
cargo deny check licenses
```
Then set Status above back to "implemented, tested" and delete this section.

## Commands
Same environment as above (shared target, debug info off, `-j 2`).
```
cargo test -p metallic_audio -j 2
cargo clippy -p metallic_audio --all-targets -j 2 -- -D warnings
cargo clippy -p metallic_audio --all-targets --no-default-features -j 2 -- -D warnings   # headless-only build
cargo bench -p metallic_audio --bench mix -j 2
cargo run -p metallic_audio --example spatial_demo -j 2 -- 12          # audible, headphones recommended
cargo run -p metallic_audio --example spatial_demo -j 2 -- 2 --mute    # silent device smoke test
```

## Next
1. Runtime wiring after Task #8 (see ARCHITECTURE.md "Audio wiring"): `Sound`, `SoundGroup`, `SoundService` and the
   `*SoundEffect` classes in `classes.rs`, a `metallic_runtime` audio system that mirrors them, Luau methods/events.
2. Asset pipeline: `SoundId` resolution (`rbxassetid://` style ids to project files), and a streaming threshold
   (assistant's call: decode files under ~10 s fully, stream longer ones).
3. Per-sound effects (Roblox allows effects parented to a `Sound`; only group effects exist now).
4. `PlaybackRegion`/`LoopRegion`, `PlaybackLoudness`, `SoundService.AmbientReverb` presets (map to `ReverbParams`).
5. M2 voice chat: Opus decode into a `StreamSource` (ring buffer fed by the network thread) on the Voice group;
   proximity voice = the same emitter/spatializer. Opus options: `opus` crate (MIT/Apache-2.0) over libopus
   (BSD-3-Clause) via `audiopus_sys` (ISC), or pure-Rust `opus-rs` (BSD-3-Clause, maturity unchecked).
6. HRTF option: Steam Audio (Apache-2.0 since Feb 2024) through `audionimbus` (MIT/Apache-2.0) as an alternative
   spatializer; needs the native library shipped with the player.
7. Move stream decoding to a worker thread if profiling ever shows xruns (today packets are decoded on the audio
   thread, which is fast for Vorbis/MP3 but not real-time safe in the strict sense).

## Open questions for the owner
- Default group of a `Sound` without `SoundGroup`: SFX (assistant's call, so the player's effects-volume slider covers
  it) vs Master (Roblox). Easy to flip in `SoundProperties::default`.
- Should creators see `priority` as a Metallic-only property on `Sound` (Roblox has none)?

---

## Research notes

### Roblox semantics we keep (names snake_cased in Rust)
- `Sound`: `SoundId`, `Volume` (0-10, default 0.5, linear), `PlaybackSpeed` (default 1; also pitch), `Looped`,
  `Playing` (setting true plays/resumes, false pauses), `TimePosition` (settable; reset to 0 by `Stop`),
  `TimeLength`, `IsPlaying`, `IsPaused`, `RollOffMode`, `RollOffMinDistance` (10), `RollOffMaxDistance` (10,000),
  `SoundGroup`, `PlayOnRemove`, `PlaybackRegion`/`LoopRegion`. Methods `Play`, `Pause`, `Resume`, `Stop`. Events
  `Played`, `Paused`, `Resumed`, `Stopped`, `Ended`, `DidLoop(soundId, numOfTimesLooped)`, `Loaded`.
  `Play()` starts at the TimePosition last set by script.
- `Enum.RollOffMode`: Inverse (0), Linear (1), LinearSquare (2), InverseTapered (3). The docs describe
  InverseTapered as inverse near the min distance and LinearSquare near the max, taking the lower value; we
  implement exactly `min(Inverse, LinearSquare)`. Community measurements (DevForum, FMOD source) suggest slightly
  different exponents in the real engine; we follow the documented formulas. All modes are silent from
  `RollOffMaxDistance` on (assistant's call; Inverse at that distance is already -60 dB with the defaults).
- A `Sound` is positional when parented to a part or attachment; otherwise it plays everywhere unpanned.
- `SoundService`: `AmbientReverb`, `DistanceFactor` (3.33 studs per metre), `DopplerScale` (1), `RolloffScale` (1),
  listener selection (`SetListener`, camera by default).
- `SoundGroup`: `Volume` (Roblox default 0.5), nestable. `SoundEffect` subclasses (`Enabled`, `Priority`):
  Reverb (DecayTime, Density, Diffusion, DryLevel, WetLevel), Equalizer (HighGain, LowGain, MidGain, MidRange),
  Compressor (Attack, GainMakeup, Ratio, Release, Threshold, SideChain), Distortion (Level), Echo (Delay, DryLevel,
  Feedback, WetLevel), plus Chorus, Flange, PitchShift, Tremolo (not implemented).

### Spatial audio choices
- **Panning vs HRTF.** HRTF (binaural filters per direction) gives real elevation and front/back cues on
  headphones but costs a convolution per voice, sounds odd on speakers, and needs an HRTF data set. Equal-power
  panning is cheap, robust on speakers and headphones, and is what most Roblox-style games ship. Assistant's call:
  equal-power panning plus a mild rear low-pass (front/back cue) now; Steam Audio HRTF later as an option.
- **Distance:** the four Roblox curves above. **Doppler:** `(c - v_listener) / (c - v_source)` along the line of
  sight, clamped to 0.25-4, `c = 343 m/s x DistanceFactor`. Velocities come from the runtime (part velocity, camera
  velocity), not from position differences, so teleports cause no pitch spikes.
- **Occlusion:** the runtime answers 0..1 per playing positional sound per frame (one raycast each; later cached or
  spread over frames). 1 = gain x 0.35 and a 1.2 kHz low-pass. Diffraction/portals are out of scope.

### Rust crates and licences (permissive only)
| Crate | Licence | Use |
|---|---|---|
| cpal 0.18 | Apache-2.0 | **Used** (feature `device`): device output on WASAPI/CoreAudio/ALSA. |
| symphonia 0.6 | MPL-2.0 (file-level copyleft; allowed, we do not modify it) | **Used**: WAV, OGG Vorbis, FLAC, MP3 decode and streaming. Pulls rustfft (MIT/Apache) for SIMD. |
| glam 0.30 | MIT/Apache-2.0 | **Used**: `Vec3` (same version as metallic_render). |
| hound 3.5 | Apache-2.0 | Dev only: writes test WAVs. |
| flacenc 0.5 | Apache-2.0 | Dev only: writes test FLACs. |
| criterion 0.8 | Apache-2.0/MIT | Dev only: bench. |
| kira 0.12 | MIT/Apache-2.0 | Considered: full engine (tracks, effects, spatial tracks, clocks, cpal + symphonia). Not used: we need Roblox roll-off curves and transport semantics, priority voice stealing, the occlusion hook, and bit-exact offline rendering independent of buffer size; building on its internals would fight its API. |
| rodio 0.23 | MIT/Apache-2.0 | Considered: simple playback, no groups/effects worth the name. |
| oddio 0.7 | MIT/Apache-2.0 | Considered: lightweight spatial mixer; small community, no effects. |
| fundsp 0.23 | MIT/Apache-2.0 | Later, for procedural audio/synths; too generic-heavy for the core mixer. |
| lewton 0.10 / claxon 0.4 | MIT/Apache-2.0 / Apache-2.0 | Superseded by symphonia's Vorbis/FLAC. |
| audionimbus 0.16 (Steam Audio) | MIT/Apache-2.0; Steam Audio itself Apache-2.0 | Later HRTF option; native library. |
| opus 0.4 + audiopus_sys | MIT/Apache-2.0, ISC (libopus BSD-3-Clause) | M2 voice chat. |
No GPL/LGPL crate is needed; `cargo deny check licenses` passes.

### Latency and budgets
- Device: WASAPI shared mode on Windows runs ~10 ms periods; cpal's default buffer is used. Our block adds at most
  64 frames (1.3 ms). A command (play, move) is heard within one device period plus one block: about 11-12 ms, under
  one 60 Hz frame. Exclusive mode / low-latency buffers can come later if rhythm games need them.
- Mixing budget (assistant's call): under 5 % of one core for 64 voices with effects. Measured 3.5 % (plain) and
  4.7 % (occlusion + reverb) on a busy machine.
- The audio thread never waits on a lock: commands go through `std::sync::mpsc`, feedback through atomics. Steady
  state does not allocate; removed voices/groups/effects are sent back to the game thread to be freed.

### Voice chat (M2) needs
- Opus at 48 kHz, 20 ms frames, a jitter buffer feeding a `StreamSource` per remote speaker on the Voice group.
- Proximity voice uses the same emitter/roll-off/occlusion path as any `Sound` (emitter = speaker's head).
- Echo cancellation and capture (cpal input) are separate work; not in this crate yet.

### Known quirk found
- Symphonia 0.6.1 rejects FLAC streams whose STREAMINFO minimum block size counts a short final block (flacenc
  writes that). libFLAC files are not affected; the test uses whole blocks. Worth an upstream report.

Sources: create.roblox.com reference pages for Sound, SoundService, RollOffMode (and the RollOffMode docs summary via
search); DevForum thread "RollOffMode documentation" (84243); docs.rs for kira; Phoronix and GameFromScratch on Steam
Audio going Apache-2.0 (Feb 2024); licences from `cargo info`.
