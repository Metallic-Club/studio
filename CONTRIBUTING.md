# Contributing

Everything in [`AGENTS.md`](AGENTS.md) applies to people too. This page covers the mechanics.

## Issues

- Work happens through issues. Labels tell you **type**, **area**, **priority** (`P0`–`P3`) and **status**.
- Only `status:ready` issues may be started. The conductor flips `status:blocked` → `status:ready` once everything in
  the issue's "Blocked by" line is closed.
- An issue that is too big to finish in one PR is split before it starts.

## Branches and commits

- Branch name: `<issue#>-<slug>`, for example `12-core-values`.
- Commit message: `<area>: <what changed> (#N)`, in the imperative, for example `core: add CFrame inverse (#12)`.
  The body explains why, when that isn't obvious.
- Keep history readable. PRs are squash-merged, so the PR title becomes the commit on `main`.

## Pull requests

- One issue per PR, with `Closes #N` in the description.
- Fill in the template checklist. CI must pass.
- Keep the diff to what the issue asks. Follow-up work gets its own issue.

## Decisions

A change to the architecture (crate boundaries, the object model, scripting, rendering, file formats) needs a
decision record first:

1. open an issue of type `type:decision`;
2. once the owner accepts it, add `docs/adr/NNNN-title.md`.

Decision records are never rewritten after acceptance; a new record supersedes an old one.
