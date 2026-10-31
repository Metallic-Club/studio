# #42 M1 crate and API feasibility (branch `studio/42-m1-crate-feasibility`)

## Goal
A sourced research doc that gives the remaining M1 tasks crate choices, API names and risks, and checks the merged
tasks against the same sources. Deliverable: `docs/research/2026-10-30-m1-crate-feasibility.md`. No dependency or
interface change.

## Done
- Research doc written against `origin/main` at `96aceb0` (waves 1-2, Task 8, #266, #270 and the Launcher plan merged). It has sections for Studio
  (9-11), Task 13, Tasks 14/16, Task 15, Tasks 6-7 vs merged code, and Task 5 (dropped: implemented by the lead). It
  also has the licence/version table, findings and risks, owner decisions, follow-up issue titles and an indexed source list.
- Sourced inputs: four local research notes (not in git) in
  `S:/Projects/Active/Metallic/research_notes/Metallic M1 crate feasibility/`. Extra crates.io API checks and local
  crate-source reads (velopack 1.3.39, keyring 3.6.3) were made 2026-10-30.

## Next
- Finish the publication checks, open the PR (`Closes #42`) and set #42 to `status:review`.
- The follow-up issues listed in section 10 of the doc are not created. File them when the lead or conductor agrees.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

## Open questions
- These need the owner: the CDLA exception for `webpki-roots`, OFL/Ubuntu fonts allowed for all crates, GitHub App
  registration and the plan/spec wording, the release repo for `stable`, and code signing (doc section 9).
- Doc findings against baseline Task 13 code (velopack prerelease pin, keyring target
  and roaming) are for the lead to accept or reject.

## Publication continuation

The original author explicitly released the claim in issue #42 at 2026-10-30 23:09 UTC. Codex root continued
the published branch at 05c5823930c4b80ac898714745f914ca973fc8f0; the cited 96aceb0 is the research's code baseline,
not that branch head. A normal merge pins current source 5c84cf7566901c73bd9de3621ab6ad384add8030. New task edits
remain limited to this progress note and the research document; the earlier observations stay dated to 96aceb0.

The accuracy pass records Task 10's implementation, removes the contradictory remaining ten-release download
claim because TaggedGithubSource pins the selected tag, and corrects the undo recommendation: scene serialization
alone cannot restore destroyed generational IDs. Current command history parks live instances and reattaches
them, disposing of them when history releases the entry. License findings describe the historical evidence gap
without asserting that no owner ever approved the policy. No dependencies or public interfaces change.

The original source index is preserved. Selected primary API pages for mlua 0.12.2, Velopack 1.2.161 and puffin
were checked again during the publication pass; this is not a new audit of every historical citation or an update
of every candidate version. Fresh workspace prebuild, strict all-target Clippy, licenses, format and task-range
whitespace checks passed. The full actual workspace suite is queued in the shared Lua window before publication.

The source-label integrity check found 97 defined entries, with no missing references or duplicate definitions.
This checks document structure only; it does not establish that every historical external page is unchanged.

An independent bounded primary-source debugger audit corrected the original pause design: hook-pushed return
tags do not survive Luau's hook stack restoration, the scheduler does not strongly retain unknown-yield threads,
and protected-error pauses need a separate adapter for LUA_BREAK. The document now labels out-of-band state plus
zero-result yield as an unexecuted candidate, adds GC/error prototype cases and records immutable source pins.
A final read-only review found no substantive contradiction; its ordinary-yield-status wording clarification was
applied. No debugger behavior or new performance result is claimed. The tested crates tree is
0178bb34d1e58cb44cfd78b01c26f438b252d6e5, exactly equal to main 5c84cf7; all task changes are documentation.
