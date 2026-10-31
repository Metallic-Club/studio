# #344: remaining unbounded C calls in the Luau sandbox

Branch `studio/344-sandbox-audit`. Code: the `STRING_LIMITS` Lua chunk in `crates/metallic_script/src/vm.rs`;
tests at the end of `crates/metallic_script/tests/vm_round2.rs`.

## Method
Timed each C library call on worst-case input in a plain Luau state of this build, memory capped at 256 MB.
The vendored Luau is compiled with `opt-level = 2` also in dev builds (`Cargo.toml`, `[profile.dev.package.luau0-src]`),
so these numbers are close to a release build.

Strings: the largest string a script can hold is about 128 MB (`string.rep` makes 64 MB at most, and one
`s .. s` doubles it; a second doubling runs out of the 256 MB memory limit). Every bound below was
checked at that size, not at 64 MB.

## Fixed
| Function | Worst case before | Fix |
|---|---|---|
| `table.concat` (millions of elements) | 2.5 s | ranges over 8192 elements join chunk by chunk; the Lua loop between chunks is interruptible. Chunking follows the range the C function walks: bounds are read as it reads them (numbers and numeric strings truncated toward zero), so `#t + 0.5` is chunked too. NaN, infinite and 32-bit-overflowing bounds are converted by the platform: for the end of the range the C function is probed (empty range or largest integer); a start bound that overflows makes the C call cheap either way. Results and errors (text, index, position) match the plain function (compared in a test, including fractional, string and overflowing bounds). Peak memory is the parts plus the result, about what the C function's own buffer needs |
| `string.pack`, `packsize`, `unpack` (64 MB format) | 0.2 to 0.65 s | format strings over 64 KB are refused |
| `string.pack("c200000000", "")`, 300 x `c999999` | padding until memory ran out | `string.pack` refuses a format whose `c<n>` items add up to more than 16 MB (every other item is at most 16 bytes, so 64 KB of format cannot reach it) |
| `string.upper`, `lower`, `reverse`, `utf8.len`, `utf8.offset` (128 MB) | 0.1 to 0.4 s (`upper`: 385 ms) | strings over 16 MB are refused (`upper` at 16 MB: about 50 ms) |
| `string.split` (128 MB of `a,`) | 0.6 s and a memory error | subjects over 4 MB are refused (4M pieces: about 0.3 s at 70 ns a piece) |

Wrappers run the C function directly. An argument error of a wrapped function (for example
`string.upper({})`) therefore reads `[engine]:N: ...` instead of the caller's position; `table.concat` is
the exception (its errors are exact). Only misuse hits this.

## Accepted bounds (measured, 128 MB string or 256 MB memory)
| Function | Worst case | Why it is left |
|---|---|---|
| `table.insert(t, pos, v)` at the front of a table with millions of elements | about 0.25 s at 8M elements | one shift. A Lua wrapper made every `table.insert(t, v)` 2 to 2.5 times slower, and a shift in chunks is not atomic: a timeout between chunks leaves a duplicated value and loses `v`. So the C function is kept as it is |
| `table.remove(t, pos)` | 19 to 26 ms at 8M elements | cheap |
| `string.format('%q', s)` and a long literal format string | about 0.2 s per 64 MB, 0.4 s at 128 MB | wrapping `string.format` would slow every call of a hot function |
| `tonumber(s)` and arithmetic coercion of a 128 MB digit string | about 0.2 s | `tonumber` is hot; coercion in `+` cannot be wrapped at all |
| `string.rep` | 45 ms for 120 MB; an empty string with a huge count returns at once | result capped at 64 MB |
| `buffer.*` | at most one memset or memcpy of the buffer (200 MB: 8 to 40 ms) | |
| `table.create`, `table.clone`, `table.freeze`, `table.find` | 12M elements: 65 ms; clone 8M: 23 ms; find 8M: 55 ms | one pass over a table limited by the memory cap |
| `string.byte`, `char`, `table.unpack`, `select`, `utf8.codepoint`, `utf8.char` | | bounded by the C stack limit (about 8000 values) |
| `string.sub`, `len`, plain `find` | 22 ms for 64 MB | one pass; plain `find` is covered by #235 |
| `utf8.codes` | | O(1) per step |
| `table.move` | | covered by #275 |
| `string.find/match/gmatch/gsub` | | Rust matcher with steps and deadline (#166, #171) |
| `math.*`, `bit32.*`, `os.clock/time`, `type`, `rawequal`, ... | | O(1) |
