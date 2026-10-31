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

Implemented source, not yet executed:

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

Next: independent source review, focused net tests, then mandatory full workspace
gates after coordinator disk/CPU approval. No Cargo has run at this checkpoint;
the coordinator paused compilation for disk recovery. Use guarded lane116 and
four jobs. Keep actual Lua/GPU suites serialized. No own merge or force push.
