# Headless GPU test diagnostics

Issue #353 supplies test diagnostics for the unresolved crashes in #276. It does
not establish a renderer, wgpu or driver defect, and it does not fix the crash.

The four headless, four lighting-preset and seven post-chain integration cases use
test-only wrappers. Each test still creates its own context; no shared device,
test mutex, extra GPU poll, backend-default change or assertion relaxation is
introduced. Only `RenderError::NoAdapter` may report an unavailable adapter and
return. Device, readback and unexpected setup errors fail. Validation panics are
not caught by the wrappers.

With `--nocapture`, stderr includes process/test identity and Rust operation
boundaries: context request, actual adapter details, renderer construction,
sync/render/capture, and handle drops. A completed handle-drop message means the
Rust drop returned; it does not prove native allocations were finally freed or
that background driver work ended. Logging itself can affect scheduling. These
diagnostics are not a memory-safety proof. Captured output missing after a native
abort does not prove that initialization failed.

`METALLIC_RENDER_TEST_BACKEND` accepts exactly `default`, `dx12`, `vulkan` or `gl`.
Unset/default preserves the production constructor's behavior. An invalid value
fails before requesting a context. The dedicated GL post-chain case always uses
explicit GL, irrespective of this test-only override. Compare the logged actual
backend and driver instead of inferring them from the request.

wgpu30.0.1's `Instance::default()` does not opt into descriptor environment
overrides. Setting `WGPU_BACKEND` alone therefore does not select this test's
backend. Environment control requires the corresponding
[`InstanceDescriptor` environment APIs](https://docs.rs/wgpu/30.0.1/wgpu/struct.InstanceDescriptor.html).
[`Instance` need not stay alive](https://docs.rs/wgpu/30.0.1/wgpu/struct.Instance.html);
its local lifetime alone is not evidence of a renderer lifetime bug.

## Bounded reproduction

Coordinate a GPU window with the other workers. Build the owned worktree through
`tools/cargo_lane.py` with `test -p metallic_render --test headless --no-run
--message-format=json --locked -j 4`. Keep the target lane reserved; do not clean,
build or hand it to another worktree during the matrix. Select the `executable`
from Cargo's `compiler-artifact` for the headless test target. Checkpoint the
source before execution; the runner refuses a dirty worktree. It does not prove
that an arbitrary supplied binary was built from that source, so retain the
build log and the guarded ownership record too.

Run `python -I tools/diagnose_headless.py --executable <owned-headless-executable>
--output <new-external-evidence-directory>`. The fixed order is `1,4,4,1,1,4`
test threads: three balanced fresh-process pairs, with no artificial load. Each
child receives `--nocapture` and a 120-second timeout. Timeout requests termination
of that owned process and reaps it before proceeding; forced termination of the
runner, or a repeated interruption during cleanup, is outside this cooperative
protocol. No unrelated process is stopped. The output directory must not already
exist, and executable hashes are rechecked before every trial.

`results.json` records source HEAD, executable hash, requested test override,
actual adapter lines per trial, raw exit status, signed/hex32 representation and
POSIX signal information separately. A status-like Windows value is only a
candidate native status; ordinary exits139/127 are not relabeled as native crash
proof. Full stdout/stderr are preserved for each child. The runner stops at any
nonzero exit or timeout instead of hiding it behind a successful rerun.

Exit0 means all six children returned zero and all four GPU contexts were observed
in each. Its outcome is deliberately `no_crash_observed`, not root cause or a
validated fix. Missing/partial adapter coverage returns exit2 with an inconclusive
outcome, even if Rust's harness reported success. A child failure/timeout returns
exit1. Keep evidence outside git and inspect logs for private paths before sharing.

If a native failure occurs, preserve the first status/logs and obtain a stack or
dump scoped to that owned test executable using available native debugging tools.
`RUST_BACKTRACE=1` can help Rust panics but is not a native heap-corruption stack.
Do not change global Windows crash settings or assign a driver cause without
evidence. Any controlled CPU-load/backend matrix comes after reviewing the first
bounded run; none is implicit in this tool.
