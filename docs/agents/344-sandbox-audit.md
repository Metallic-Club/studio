# #344: remaining unbounded C calls in the Luau sandbox

Branch `studio/344-sandbox-audit`. Code: the `STRING_LIMITS` Lua chunk in `crates/metallic_script/src/vm.rs`;
tests at the end of `crates/metallic_script/tests/vm_round2.rs`.

## Method
Timed each C library call on worst-case input in a plain Luau state of this build (the C is built
unoptimised in debug runs, so these numbers are about 3x a release build), memory capped at 256 MB.

## Fixed
| Function | Worst case before | Fix |
|---|---|---|
| `table.concat` (millions of elements) | 2.5 s | ranges over 8192 elements join chunk by chunk; the Lua loop between chunks is interruptible; same result and errors |
| `string.pack`, `packsize`, `unpack` (64 MB format) | 0.2 to 0.65 s | format strings over 64 KB are refused |
| `string.upper`, `lower`, `reverse`, `utf8.len` (64 MB) | 0.1 to 0.26 s | strings over 32 MB are refused |

## Audited, left as is
| Function | Why it is safe enough |
|---|---|
| `string.rep` | result capped at 64 MB; an empty string with a huge count returns at once (checked) |
| `string.format` | width and precision are limited to 2 digits by Luau; a 64 MB format string of literals takes about 0.16 s, one memory pass, and cannot be larger than 64 MB (`string.rep` cap) |
| `string.byte`, `char`, `table.unpack`, `select`, `utf8.codepoint`, `utf8.char` | bounded by the C stack limit (about 8000 values) |
| `string.sub`, `len`, plain `find` | one pass or O(1); plain `find` is covered by #235 |
| `tonumber` on a 64 MB digit string | one pass, about 0.1 s |
| `utf8.offset`, `utf8.codes` | O(1) per call or per step |
| `buffer.*` | one memset/memcpy of at most the buffer size (200 MB measured: 8 to 40 ms) |
| `table.create`, `table.clone`, `table.freeze`, `table.find` | at most one pass over a table limited by the memory cap (12M elements: 65 ms; clone 8M: 23 ms; find 8M: 55 ms) |
| `table.insert(t, 1, v)`, `table.remove(t, 1)` | one shift over the table: 8M elements took 220 ms (insert) and 19 ms (remove). Not wrapped: `table.insert(t, v)` is hot and a Lua wrapper would slow every append. A script can still lose up to about 0.2 s (debug build) in one call |
| `table.move` | covered by #275 |
| `string.find/match/gmatch/gsub` | Rust matcher with steps and deadline (#166, #171) |
| `math.*`, `bit32.*`, `os.clock/time`, `type`, `rawequal`, ... | O(1) |

## Open
- `table.insert` at the front of a table with millions of elements (above).
