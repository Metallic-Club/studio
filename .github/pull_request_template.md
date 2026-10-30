Closes #

## What changed
<!-- One short paragraph, then bullets if needed. -->

## How tested
Paste the real results (or write "not run" and why).
- `cargo test --workspace`:
- `cargo clippy --workspace --all-targets -- -D warnings`:
- `cargo deny check licenses`:
- Manual checks:

## Screenshots
<!-- Required for any visual change. Say you looked at them. Otherwise write "none". -->

## Checklist
- [ ] Issue linked above and set to `status:review`
- [ ] Progress note `docs/agents/<issue#>-<slug>.md` is current
- [ ] No secrets, server addresses, user names, keys or tokens
- [ ] New dependencies are permissive-licensed (`cargo deny` passes)
- [ ] Brand rules kept (name "Metallic", graphite/silver/restrained violet, no neon/glow/shields/starbursts)
- [ ] Rebased on `main`; no unrelated changes
