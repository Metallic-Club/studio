# 430: bounded transport receive delivery

Goal: provide a strict count/payload-byte poll primitive for the future #336
driver without changing wire encoding, authentication, runtime roles or hosting.
Claimed by Codex engine_research on branch `studio/430-bounded-poll`, frozen base
`8699192791825af0c8c44edb9d833339440cc508`.

A5 #297 and A6 #328 are closed. Their remote heads are not literal ancestors of
main's squashed history, but the selected main transport files/tests match A5
`6ba838677c34d4cb6c8c1fdc15ac271c8ae7ff13`, and all net source matches A6
`2e3ad637bb12e3dacbfa0dde7a64f1c763ff4bbb`. Live open issue/PR searches found no
overlapping transport claim. The external read-only audit is separate from the
preserved #336 proposal.

Implementation at the first source checkpoint, before execution:

- Required Connection::poll_bounded and PollBudget/Report/Stop. No default
  unrestricted fallback; old poll callers are preserved, while external trait
  implementors must explicitly add the method.
- Shared ReceiveState enforces event/byte limits and holds one indivisible FIFO
  head. All variants, including UTF-8 close reason text and zero-byte events,
  have explicit accounting. Output prefixes and LinkStats retain their semantics.
- Memory inbox is a deque, avoiding backlog-linear front removal. QUIC uses the
  same bounded state over its channel. Local close discards pending delivery,
  including a head retained before an already-queued peer close.
- Deterministic memory/channel controls, an always-ready producer contract test,
  and a bounded-deadline public QUIC smoke cover the new operation. Existing
  transport/wire tests are retained.

## Validation and final source

Initial source: `795f959ec2fb6b1fc9ba06ec4fad731094fc091a`. Guarded lane116 and
four build jobs were used; actual Lua/GPU work used a reserved sole window.

- Focused net suite: 69 passed, zero failures and no ignores across five groups
  (13 library, four new bounded public controls, 28 existing transport and 24
  wire cases). This includes actual loopback QUIC and deterministic queue/memory
  cases; the UTF-8 fixtures are actual U+00E9 and pass their two-byte assertions.
- Full workspace prebuild passed. Full locked serial workspace execution passed
  2,024 tests, zero failures and eight upstream ignores across 148 result groups,
  including doc tests and all three complete Parkour routes.
- Initial strict full Clippy failed on a test assertion's unnecessary clone.
  The same form in a private control was corrected too, without an allowance:
  `assert_eq!(out, [prefix.clone()])` became an exact borrowed-slice comparison.
  Child `6653d5ea094efcf93f16611aff9ce99b315072cf` changes only those two test
  assertions. Every other file is identical; the QUIC production prefix before
  its test module is byte-identical. The failed log, patch and production hashes
  are retained. The earlier full-run result remains pinned to 795f, not relabeled.
- On 6653, all 17 affected library/public controls passed. Full strict all-target
  Clippy passed in 42.78s, as did workspace format and pinned-base whitespace.
  The license gate passed; no dependency, manifest or lockfile changed.
- Three independent read-only reviews (worker coordinator, backend and core
  workers) found no actionable source issue at 795f. The coordinator also checked
  the exact assertion-only child. No test budget, tolerance or ignore changed.

The coordinator waived another entire local workspace execution for the two
equivalent assertion forms: final focused/strict proof and production equality
are explicit, and exact-head CI reruns the corrected whole workspace. This is a
validation decision, not repository acceptance or permission to merge this PR.

Disk recovery used verified, locked native Cargo package cleanup, never recursive
filesystem deletion. After all proof was saved, the released lane's 21 first-party
dev packages were cleaned (7.2 GiB), preserving third-party caches, source, logs
and named benchmark domains. Lane116 is idle/released with no retained binary
requirement. Detailed source/exit/log/hash evidence is under external
`integration-evidence/430-bounded-poll`; eligibility and cleanup provenance remain
under `integration-evidence/transport-poll-audit`. No own merge or force push.

Next: publish the docs-only final child and inspect exact-head CI. No additional
local build or source change is planned.
