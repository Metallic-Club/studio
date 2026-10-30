# #42 M1 crate and API feasibility (branch `studio/42-m1-crate-feasibility`)

## Goal
A sourced research doc that gives the remaining M1 tasks crate choices, API names and risks, and checks the merged
tasks against the same sources. Deliverable: `docs/research/2026-10-30-m1-crate-feasibility.md`. No dependency or
interface change.

## Done
- Research doc written against `origin/main` at `96aceb0` (waves 1-2, Task 8, #266, #270 and the Launcher plan merged). It has sections for Studio
  (9-11), Task 13, Tasks 14/16, Task 15, Tasks 6-7 vs merged code, and Task 5 (dropped: implemented by the lead). It
  also has the licence/version table, findings and risks, owner decisions, follow-up issue titles and 93 sources.
- Sourced inputs: four local research notes (not in git) in
  `S:/Projects/Active/Metallic/research_notes/Metallic M1 crate feasibility/`. Extra crates.io API checks and local
  crate-source reads (velopack 1.3.39, keyring 3.6.3) were made 2026-10-30.

## Next
- Open the PR (`Closes #42`) when the lead asks; set #42 to `status:review`.
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
- Doc findings against merged Task 13 code (10-release window in `download`, velopack prerelease pin, keyring target
  and roaming) are for the lead to accept or reject.
