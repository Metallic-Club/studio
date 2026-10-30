# #138 Litha design doc

Goal: research and design doc for Litha, the in-Studio AI teacher (epic #20, M7). Docs only.

Done:
- `docs/research/2026-10-30-litha.md`: modes, surfaces, tool interface, caching, model and cost, backend proxy,
  rate limits, child safety and privacy, prompt-injection defences, evals, failure modes, phases, proposed issues.
- Sources fetched 2026-10-30; model facts from the `claude-api` skill reference (cached 2026-10-06).

Next:
- Owner review of the assistant's calls (doc section 13), especially the age policy and Opus 5.5 at launch.
- File the proposed issues (doc section 16); issue 1 belongs in `Metallic-Club/plateform`.

Open questions:
- `docs/luau-api.md` does not exist yet (Task 7). The API index generator depends on it.
- Legal review of COPPA, UK code and EU AI Act Article 50 points before any teen or under-13 launch.
- Anthropic request-metadata field name for the per-account hash: confirm when building the gateway.
