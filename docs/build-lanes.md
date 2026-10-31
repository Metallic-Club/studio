# Local Cargo build lanes

Use the standard-library guard when reusing a target directory between worktrees.
Run this from the worktree you want to verify, with Python 3.11+ and Cargo on PATH:

```powershell
python -I tools/cargo_lane.py --target-dir "$env:LOCALAPPDATA\Metallic\cargo-target\worker1" -- test --workspace -j 4 -- --test-threads=1
```

The target must be a dedicated artifact directory outside the source workspace.
Choose a separate lane for each worker that needs to build concurrently. A second
guarded command using the same lane waits for the first command to finish, including
its test executables. Keep that lane for subsequent commands in the same worktree:

```powershell
python -I tools/cargo_lane.py --target-dir "$env:LOCALAPPDATA\Metallic\cargo-target\worker1" -- clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

On macOS/Linux, use an absolute path such as `"$HOME/.cache/metallic/cargo/worker1"`
for `--target-dir`. The workspace defaults to the parent of this script's `tools/`
directory; `--workspace PATH` explicitly selects another workspace root.

## What the guard checks

The reproduced problem was a stale local `metallic_core` artifact when different
Windows/OneDrive worktrees reused one target directory with Cargo 1.99.1. Both
concurrent and sequential reuse produced a missing-method error despite the method
being in the worktree source. A fresh target or a clean of local workspace packages
restored correct validation. This is local evidence; it does not establish an
upstream Cargo defect on every host.

`tools/cargo_lane.py` holds a process lock for metadata discovery, any cleanup, and
the complete build/test command. It records a versioned owner containing the resolved
workspace root, exact target directory, current local workspace package names and
selected profile/target architecture (including inherited `CARGO_BUILD_TARGET`).
Cargo configuration files, including their recursive includes, are fingerprinted
so edits to an inherited `build.target` also invalidate ownership. Their contents
are never recorded in the owner file.
The owner and lock live in the artifact lane, outside the source checkout. They are
local cache bookkeeping and must not be committed. Ownership means source provenance,
not a passing test result.

A missing, corrupt, different or outdated owner triggers one `cargo clean` with a
`--package` argument for every local workspace member from `cargo metadata`. The
helper never recursively removes a lane, sources or third-party dependencies. It
publishes the new owner only after that clean succeeds. The same owner and
compilation selection skip the clean. Metadata, cleanup and build/test failures
return Cargo's actual status; a cleanup failure stops the build and keeps the previous owner.

A verified empty lane is initialized with a standard cache tag under the lock and
needs no cleanup. Existing nonempty directories without a valid Cargo cache tag
are refused before metadata, cleanup or build. The guard never marks those
directories as caches; choose a new empty lane instead. This also covers recovery
when a directory's provenance cannot be established.

The helper rejects filesystem roots, source checkout paths and existing symlink or
Windows reparse components in the target path, lock path and owner path. It fixes
both Cargo's target and intermediate build directories to the lane, passes argument
arrays without a shell, and rejects forwarded target, config, manifest and working
directory overrides. Supported commands are `build`, `check`, `test`, `clippy`,
`bench`, `run` and `doc`. Release, custom profile and target selections are forwarded
to the package clean; switching one of these selections also requires a clean.
Use separate short flags (`-q -r`, not `-qr`); attached `-j4`, `-pNAME`, `-Ffeatures`
and repeated `-v` remain supported. Arguments after `--` remain literal test/application
arguments for `test`, `run` and `bench`. Clippy forwards that tail to the compiler,
so its tail accepts only lint controls (`-A`, `-W`, `-D`, `-F`, `--cap-lints`) and
their lint names/levels. Compiler output/codegen flags are rejected. Other Cargo
commands, including unrestricted `clean`, are excluded.

The guard cooperates with other guarded invocations. Raw Cargo, an IDE's background
build or another tool bypasses its lock: those processes must use another lane or
finish before this lane is used. Do not modify a lane's lock/state files while it is
in use. Ordinary Ctrl-C keeps the lock until Cargo exits. Forcibly terminating the
guard can leave child processes running; wait for those processes to exit before
reusing or recovering that lane. The helper is not a sandbox for build scripts or
test executables, and it does not authenticate arbitrary files inside a cache.
Compiler wrappers, runners and configured rustflags remain trusted configuration;
do not configure them to share compiler output outside the guarded lane.

## Recovery

After any unguarded use, unexplained artifact mismatch, or uncertain cache provenance,
stop other users of the lane and run a forced package-only clean plus verification:

```powershell
python -I tools/cargo_lane.py --target-dir "$env:LOCALAPPDATA\Metallic\cargo-target\worker1" --reset -- test --workspace -j 4 -- --test-threads=1
```

Do not delete the lane root or the lock file as a recovery step. No global Git or
Cargo settings are needed. Same-worktree source edits and rebases rely on Cargo's
normal invalidation; `--reset` is available when that provenance is uncertain.

## Tests and references

`python -I tools/test_cargo_lane.py` uses temporary directories and a stub Cargo
executable. It covers owner transitions, metadata-derived package selection, cleanup
failures, propagated test failures, unsafe locations/overrides, Windows junctions,
and two real processes competing for the lane through test completion. Ordinary
symlink creation is skipped on Windows hosts without the necessary privilege;
junction rejection is tested separately there. A real std-only Cargo probe checks
fresh-lane bootstrap, same-owner reuse and a release selection; it is skipped only
when Cargo is unavailable. That probe also uses a Japanese-named workspace with
Python's UTF-8 mode disabled, exercising explicit UTF-8 metadata decoding on Windows.
CI runs the suite on Windows and Linux.

Cargo documents the distinction between final and intermediate artifacts in
[Build Cache](https://doc.rust-lang.org/cargo/reference/build-cache.html), and the
target/build directory settings and environment overrides in
[Configuration](https://doc.rust-lang.org/cargo/reference/config.html). These describe
the location controls used by the helper; the ownership rule addresses our observed
worktree mismatch.

The [clean command](https://doc.rust-lang.org/cargo/commands/cargo-clean.html) documents
its package/profile/target selectors. An isolated std-only probe on Cargo 1.99.1
confirmed that a default package clean left the probe's release artifact present;
the helper therefore explicitly matches cleanup to the requested profile and target.
