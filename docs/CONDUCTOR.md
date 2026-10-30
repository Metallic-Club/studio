# Conductor baton

One AI session at a time is the **conductor**: it keeps the whole project moving. Any AI can hold the baton (a Claude
Code session, ChatGPT/Codex, another assistant). Owner's rule (2026-10-30): "if you run out of usage, make another AI
the conductor; they have to claim it and pass it on before they run out."

## Where the baton lives
GitHub issue **"Conductor baton"** (label `conductor`, pinned). Its comments are the baton log. The latest
`CLAIM` comment without a later `RELEASE` names the current conductor.

## Conductor duties
- Keep `docs/STATUS.md` true: what is merged, what is in progress, what is next.
- Pick and start the next unblocked issues (dependency graph in `CLAUDE.md`), at most 4-5 workers at once.
- Get every task reviewed (spec + quality), then merge green PRs into `main` (owner allows push and merge; never
  force-push).
- Record every decision made on the owner's behalf as a `Ruling:` line in the baton issue.
- Watch your own usage and hand over **before** you run out.

## Claim
1. Read the baton issue's last comments, `docs/STATUS.md`, open PRs and `status:in-progress` issues.
2. If the last `CLAIM` has no `RELEASE` and is less than 2 hours old (or its holder posted in the last 2 hours),
   do not take it; work on a `status:ready` issue instead.
3. Otherwise post:
   `CLAIM by <who> (<model/app>) at <UTC time>. Usage: <5-hour %>, <week %>. Plan: <next 1-3 steps>.`

## Hand over (before usage runs out)
Hand over when your 5-hour window passes 85%, your week passes 90%, your context is nearly full, or you must stop.
Post one comment:
```
RELEASE by <who> at <UTC time>.
Running now: <workers / branches / PRs and what each is doing>
Waiting on review or CI: <PRs>
Next steps, in order: <1..n>
Rulings since last handover: <list>
Gotchas: <anything the next conductor must know>
```
Then stop dispatching new work. Workers already running finish their task and report in their PR.

## No conductor?
If nobody has claimed for 2 hours, any AI with usage left may claim. Workers never wait for a conductor: they pick
`status:ready` issues and follow `CLAUDE.md`.
