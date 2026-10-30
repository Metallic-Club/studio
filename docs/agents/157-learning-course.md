# #157 Learning course design

Goal: design the interactive course (basics to advanced), the lesson format and the in-Studio lesson runner
(epic #20, M7). Docs only.

Done:
- `docs/design/learning-course.md`: principles, evidence from other courses and learning research, 10 tracks and
  53 units plus 5 capstones mapped to issues and milestones, lesson file format (TOML + Luau checks), three full
  example lessons (B1.3, S7.2, N2.3), lesson runner placement, checks engine, progress storage, offline behaviour,
  badges and the publishing path, reading level, localisation, screen readers, age policy, phased plan of 26
  proposed issues, assistant's calls, gaps.
- Sources fetched 2026-10-30 (section 12 lists fetched pages and search-only summaries separately).

Next:
- Owner review of section 10 (assistant's calls), especially "finish P3 before a first paid product".
- File the proposed issues in section 9 (item 18 belongs in `Metallic-Club/plateform`).
- Align #83's tutorial runner with the lesson runner (one runner, `tutorial.toml` as `kind = "tour"`).

Open questions:
- Networking design (#156) not merged; Multiplayer units may change.
- `docs/luau-api.md` (Task 7) and the API index (#141) are needed for `requires_api` gating.
- Under-13 sync and public badges wait for platform parental consent and legal review (as in the Litha doc).
