# #204 Preserve a session after another process rotates its refresh token

Goal: a stale refresher cannot delete a newer stored session after receiving
`bad_refresh_token`; independent auth instances recover a fresh rotated session.

## Scope and contract

- Only fake HTTP and in-memory stores in tests; no real network/credential writes.
- Existing network retry/transient-error policy remains unchanged (#206).
- SecretStore currently exposes separate get/delete, not compare-and-delete.
  Native Windows CredDelete also has no expected-value argument. A get/delete pair
  cannot safely claim cross-process atomic cleanup.
- Added optional atomic conditional-delete capability: memory backend
  implements it under its existing mutex; default/native backend declines cleanup
  without mutating. Manual sign-out still explicitly deletes. Unsupported backends
  retain an invalid entry and return the provider error rather than risk a newer
  credential. This limitation is deliberate, not cross-process serialization.

## Done

- Claimed #204, branch `studio/204-refresh-race`, dedicated worktree from `ebb94f3`.
- Read current auth/store and existing refresh tests; verified GitHub rotation rules
  and Microsoft's native delete signature against primary documentation.
- Added deterministic two-auth/shared-MemorySecretStore regression with bounded
  fake-HTTP gates. Both requests use the old token; winner persists before stale
  error returns, avoiding timing/sleep assumptions.
- Confirmed the two-auth regression fails before the fix and passes afterward.
- On provider errors, re-read storage, conditionally clean only the exact old value
  on a capable backend, re-read after cleanup and reuse an unexpired changed-refresh
  session. No unsafe get/delete fallback exists.
- Six focused tests pass: rotation, a writer between comparison and cleanup, unchanged
  invalid cleanup, expired/malformed/empty/same-refresh advanced values, unsupported
  backend retention/manual sign-out, and unchanged four-attempt network retry behavior.
- Updated module/API docs, architecture and release guidance with native keyring and
  in-flight-writer limitations. No cross-process mutex or credential-store CAS is claimed.

## Next

- CI and maintainer integration after PR delivery. Do not merge this session's PR.

## Validation

- Original deterministic two-auth regression failed, then passed after the fix.
- Focused `refresh_race` integration suite: all 6 tests passed, with no real network
  or credential-store access. Native cross-process operations were not exercised.
- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: passed, including existing
  auth/logging tests, new race tests, full input-only Parkour route, player smoke and Luau.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed (existing unused MPL-2.0 allowance warning).
- `cargo fmt --check`, `git diff --check`: passed.
- Independent read-only review found no additional actionable issues. Successful
  writes/in-flight refreshes are not cross-process serialized, as documented.
- Guarded lane ownership transition cleaned only local workspace packages before
  compilation; dependency artifacts were retained. Timed suites were coordinated
  with other agents to avoid overlapping heavy compilation.

## Primary references

- [GitHub refresh rules](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens)
- [Microsoft CredDelete](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-creddeletew)

Accessed 2026-10-30. No conditional-delete/native-CAS guarantee is inferred from these APIs.
