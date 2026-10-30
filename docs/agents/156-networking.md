# M2 networking design (issue #156, branch `studio/156-networking`)

Goal: research and a concrete design for milestone 2 networking: transport, protocol, replication, character
prediction, remotes over the wire, `metallic-server`, Studio tests, Team Create, voice, testing, phased plan.
Deliverable: `docs/research/2026-10-30-networking-m2.md`. Docs only; nothing built (disk nearly full, no cargo).

## Done
- Read the spec, ARCHITECTURE.md, the platform architecture and performance documents, the platform OpenAPI
  (`plateform` branch `platform/p1-backend`, 0.1.1), `metallic_core` journal, `metallic_physics` character controller,
  `metallic_script` remotes (branch `studio/7-luau-api`).
- Fetched about 30 sources (Roblox docs, Fiedler, Quake 3, netcode.io, Gambetta, Rocket League slides, Figma,
  Discord, RFC 9221, quinn, WebTransport, crates.io licences).
- Wrote the design document with decisions, platform API gaps, 37 proposed issues in M2a-M2e, risks.

## Next
- Lead or owner reviews the assistant's calls (section 14 of the doc) and creates the issues they accept.
- Raise the platform API gaps (doc section 1.3) with the platform team; do not edit `plateform` from this repo.

## Open questions
- Voice age policy, Team Create offline editing, first server region, max players per server (doc section 14).
