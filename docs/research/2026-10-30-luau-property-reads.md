# Instance member-key allocation (#119)

The existing `member_name` helper copied every string key into a Rust `String`.
Both `__index` and `__newindex` now retain an owning `mlua::BorrowedStr` for valid
UTF-8. The guard remains alive through world access and Lua conversion. Invalid
UTF-8 still uses replacement characters in an owned fallback. Non-string keys
remain errors, including numbers; a typed mlua string argument could introduce
numeric coercion and was deliberately avoided.

This change is independent of the proposed core borrowed-value API and VM timer.
It keeps the current owned `World::get` conversion and f64 Vector3 userdata.

## Evidence and reproduction

Baseline production source: main `88f5512e373ade3575e73b447858da84f5c37b03`.
The same test-only fixture and compiler/profile were used before and after the
key conversion change: Windows x86_64 MSVC, Rust/Cargo 1.99.1, mlua 0.12.2, Luau
0.740, release build. Source lives in
`crates/metallic_script/src/api/instance/read_probe.rs`.

```text
cargo test --release -p metallic_script -j 4 --lib valid_member_keys_do_not_allocate_owned_rust_strings -- --nocapture --test-threads=1
cargo test --release -p metallic_script -j 4 --lib property_getter_probe -- --ignored --nocapture --test-threads=1
cargo test --release -p metallic_script -j 4 --lib paired_property_getter_probe -- --ignored --nocapture --test-threads=1
```

The ordinary allocation regression warms 64 conversions, then converts the same
`Anchored` key 10,000 times. It failed against the baseline with 10,000 allocation
requests and 80,000 requested bytes, and passes with zero of each after this change.
The result is consumed through `black_box`.

The ignored diagnostic probe builds a real World and API, precompiles each Lua
function, and passes the subject directly as a function argument. Construction and
compilation are outside measurement. Each call performs 1,000 reads under the
unchanged 100 ms execution cap; 64 calls warm up, then 20 samples each time 10 calls.
The scalar path reads `p.Anchored`; the vector path reads the explicitly stored
`p.Position`. A plain Lua table using the same scalar function is a reference for
the harness, not an equivalent Instance implementation.

The test-only global allocator forwards to `System` and records allocation/realloc
requests on the current test thread using thread-local counters and a drop guard.
Counts include requests made by mlua's Lua allocator as well as Rust API work.
Bytes are cumulative requested allocation sizes, not retained heap, allocator
metadata, RSS, or process memory. Timing also includes the disabled instrumentation
check; these numbers do not describe an uninstrumented production build.

One measured batch of 1,000 real reads after warm-up:

| Path | Before requests / requested bytes | After requests / requested bytes |
| --- | ---: | ---: |
| Scalar Instance | 2,005 / 24,480 | 1,005 / 16,480 |
| Vector Instance | 3,011 / 170,640 | 2,011 / 162,640 |
| Scalar Lua table reference | 5 / 480 | 5 / 480 |

Both Instance paths remove exactly one 8-byte key allocation per read. Remaining
requests include binding and value-conversion work; this change does not claim
allocation-free property reads. The probe's separate, initially unwarmed key loop
recorded 10,001 / 80,016 before and 1 / 16 after, including one initial 16-byte
request. The warmed ordinary regression isolates the steady-state zero-allocation
key conversion from that initial bookkeeping.

Diagnostic wall time on the shared, busy development desktop:

| Path | Before p50 / p95 ns per read | After p50 / p95 ns per read |
| --- | ---: | ---: |
| Scalar Instance | 1,645.0 / 2,232.9 | 1,863.1 / 2,506.1 |
| Vector Instance | 1,832.2 / 2,367.1 | 2,613.7 / 4,188.3 |
| Scalar Lua table reference | 58.4 / 69.3 | 65.1 / 85.0 |

The after samples were slower, including the unchanged table reference. These
unpaired runs have uncontrolled host load and no confidence interval. Follow-up
paired measurements below investigate the possible conversion/lifetime cost;
the slower first samples remain part of the evidence. The aspirational
150 ns target remains unmet in this probe. Reported Lua heap readings after the
batches were 636,472 / 621,632 / 621,632 bytes before and 636,472 / 637,992 / 637,992
after (scalar/vector/table order); GC timing and live userdata differ, so these are
diagnostics, not retained-memory comparisons.

## Same-process paired comparison

A test-only thread-local switch in `member_name` selects either the exact old
owned conversion or the proposed guard conversion. The switch is absent from
production builds. Both diagnostic paths pay the same switch and allocator
instrumentation overhead. The real Instance, VM, precompiled function and member
lookup are shared, so the comparison includes the guard staying alive throughout
world access and Lua value conversion.

The paired probe warms both modes for 64 calls, then alternates AB/BA order over
40 samples, with the same 1,000 reads per call and 10 calls per sample. It also
alternates 20,000 direct key conversions to isolate their cost. Three full runs
were performed. The later two compute a ratio for each adjacent pair before
sorting; the first version only printed ratios of sorted distributions, so its
ratio output is not used here. Core/script sources in the rebased main `ebb94f3`
were unchanged from the baseline `88f5512`.

| Run | Scalar owned / borrowed p50 ns | Vector owned / borrowed p50 ns | Key owned / borrowed p50 ns |
| --- | ---: | ---: | ---: |
| 1 | 1,802.8 / 1,693.8 | 3,071.2 / 3,008.4 | 129.1 / 50.7 |
| 2 | 1,911.9 / 1,768.3 | 3,093.9 / 2,998.1 | 119.3 / 47.2 |
| 3 | 1,823.4 / 1,615.5 | 2,697.3 / 2,493.5 | 118.8 / 47.6 |

Run 2's median per-pair borrowed/owned ratios were 0.976 scalar and 0.923 vector;
run 3's were 0.919 and 0.924. The key path was consistently faster in these runs,
and no repeatable full-getter median regression appeared under this comparison.
However, vector p95 in run 2 worsened from 4,820.4 to 5,213.5 ns, while run 3
improved from 3,965.8 to 3,640.2 ns. These busy-host observations support the narrow
conversion change, without promising a tail-latency or universal speedup.

All paired runs repeated the same real-getter allocation counts in the earlier
table. The 20,000-key conversion count was 20,000 requests / 160,000 requested bytes
for owned conversion and zero for borrowed conversion. Full getter costs remain
far above the 150 ns target and should be profiled independently before larger
reflection or datatype changes.

## Primary sources and limits

- [mlua 0.12.2 string source](https://docs.rs/crate/mlua/0.12.2/source/src/string.rs):
  inspected the installed version's `to_str`, `BorrowedStr` and `BorrowedBytes`.
  The string guard owns a value reference and a strong Lua handle. Returning a
  bare `&str` from a temporary guard would lose that lifetime protection.
- [Luau performance documentation](https://luau.org/performance/): direct table
  fields have specialized lookup paths, while reflected userdata crosses a
  binding layer. Its native-vector optimization does not justify silently
  changing this API's component precision.

Compatibility regressions cover guard lifetime after the local Lua/key handles
drop, Unicode names, lossy invalid UTF-8 child lookup and assignment errors,
non-string rejection, unknown/read-only members, destroyed-instance errors and
the existing f64 Position precision. After explicitly cleaning all seven local
workspace packages in the verified owned target, all 217 workspace tests passed
(two diagnostic probes ignored and manually run), including the Parkour route.
Strict Clippy, license, formatting and whitespace checks passed.
