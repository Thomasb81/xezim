# xezim 64-bit value handling audit

Per user request: SystemVerilog logic can be wider than 64 bits; ensure
signal values that exceed 64 bits are handled correctly. Audit done before
any implementation.

## Method

Searched `src/compiler/{simulator,bytecode,jit,jit_llvm}.rs` for all uses
of `.to_u64()`, `.to_i64()`, `from_u64()`, and `as u64`. Categorised each
call site by whether the converted value can plausibly originate from a
signal or expression whose width exceeds 64.

`Value::to_u64()` semantics (`xezim-core/src/value.rs:359-371`):
**silently keeps the low 64 bits** for wide values. Returns `Some(_)`
even when width > 64. So callers that do `.to_u64().unwrap_or(0)` will
get a truncated value, not panic — the bug is silent.

`Value::to_u128()` exists (line 136) for callers that need full width up
to 128. There is no `to_u_arbitrary` — wider needs raw bit access.

## Counts (informational)

| File | `.to_u64()` | `.from_u64(` | `as u64` |
|---|---|---|---|
| simulator.rs | 128 | 117 | 113 |
| bytecode.rs | 2 | 12 | 13 |
| jit.rs | 1 | 0 | 2 |
| jit_llvm.rs | 1 | 0 | 15 |

## Categorisation

### Category A — SAFE: index/loop/shift counts (intentional u64)

Most `.to_u64()` calls are on values whose *meaning* is a small integer
(array index, bit position, range bound, loop counter, delay count).
Even if the value originated from a wide signal, only the low bits matter
because indices into a finite structure are bounded. Truncating to 64
bits is correct in these cases.

Examples (representative):
- simulator.rs:4724 `vm_regs[*idx].to_u64().unwrap_or(0) as usize` —
  array index, target is `Vec`, max ~2^32 entries, low 64 bits suffice.
- simulator.rs:5263-5264 `hi_reg.to_u64() as u32`,
  `lo_reg.to_u64() as u32` — bit positions for `RangeSelect`, fit in u32.
- simulator.rs:8090,8370 `eval_expr(d).to_u64().unwrap_or(0)` —
  delay count for `#N` timing controls; reasonable to cap at u64.
- simulator.rs:8153,8437,12630 `to_u64()` on Replication count,
  `for` step. Bounded.
- simulator.rs:9928,9965-66,10009,10053,10077-78 — `Index`/`RangeSelect`
  index/bound expressions. Bounded.
- simulator.rs:10215,10386,10397 `to_u64() as usize` — handle-table
  cursor. Bounded.

Verdict: leave as-is. Possibly add `debug_assert!(width <= 64)` in a few
places to catch programmer errors but no behavior change.

### Category B — SAFE BUT WORTH REVIEWING: comparisons / hash keys

These compare two values via `to_u64()`. If both sources are ≤64-bit they
work; if both are >64-bit they compare only the low 64 bits. Currently no
known c910 RTL hits this on wide signals.

- simulator.rs:11984 `let key = v.to_u64()` — used as a hash key for
  uniqueness in `inside`/`unique` checks. Wide signals would collide if
  their low 64 bits match. Could shadow a real difference.
- simulator.rs:12004-12008 `to_u64() < to_u64()` — used in associative
  array sort comparator. Same wide-signal risk.
- simulator.rs:15424,15441 `elements.sort_by(... a.to_u64().cmp(&b.to_u64()))` —
  array sort comparator. Same.

Verdict: replace with `Value::compare()` style methods that respect full
width and signedness. **Do not change unless we hit a c910 case.**

### Category C — DANGEROUS: signal-value reads truncated to u64

Sites where a signal_table entry or full Value is read as `u64` and used
as the *value*, not as an index. These will silently drop bits 64+ if
the signal is wider.

| Line | Code | Width-truncation risk |
|---|---|---|
| simulator.rs:13847 | `jit_load_signal: signal_table[id].to_u64()` | **YES**. Loads a signal into JIT registers. JIT operates u64-only. Must reject any block whose signal-table read targets a width>64 signal. Codex flagged this. |
| simulator.rs:14014 | `jit_load_array_elem: signal_table[eid].to_u64()` | **YES**. Same class — loads array element into JIT. |
| jit.rs:828-832 | declared u64 raw bridge | **YES** (architectural). |
| jit.rs:969-985 | `RangeSelectConst` u64-only | **YES** for >64-bit operands. |
| jit_llvm.rs (15 `as u64`) | LLVM IR builder constants | Mostly metadata (signal IDs, widths); not value bits. Mostly SAFE, but worth re-checking each occurrence. |

Verdict for JIT path: refuse to JIT any edge block that reads OR writes a
signal of width > 64, AND any block that touches an unpacked-array
element wider than 64. See "JIT" recommendations below.

### Category D — DANGEROUS: signal-value writes from u64

Sites where a u64 is converted to a Value via `from_u64(u64, width)` for
storage to a signal. If `width > 64`, `from_u64` zero-extends — that is
correct semantically (the high bits of a u64 would be 0 anyway) — so
this is only a bug if the upstream computation was supposed to produce
wider data than u64 holds.

`Value::from_u64(val, width)` zero-extends to `width`. So writing
`from_u64(some_lo, 128)` produces a 128-bit value with high 64 = 0.
That's wrong only if the upstream u64 was a TRUNCATED view of a wider
quantity.

Representative call sites in simulator.rs:
- Many constant materializations from numeric literals (Number
  expressions). For literals ≤64 bits this is correct; for wider
  literals the parser returns Value directly without going through u64
  (verified in number-eval paths).

Verdict: low priority. Audit `eval_number_static` / Number lowering to
confirm the wide-literal path bypasses u64.

### Category E — `raw_bits()` / `inline_bits()` (already 64-bit framed)

`raw_bits()` (value.rs:222) returns `(u64, u64)` for the LOW 64 bits of
val/xz. The doc says "wide signals: bits beyond 63 are not exposed
here." Callers of `raw_bits()` on wide signals must handle the tail
explicitly.

xezim already does this in some hot paths:
- simulator.rs:8733-8741 `fires_any` on wide signal: calls
  `raw_bits()` then ALSO `prev_wide.get(&sid)` for full Value compare.
  Correct.
- simulator.rs:13862-13880 `jit_inputs_have_xz`: checks raw_bits xz != 0,
  then explicitly walks `bits` for width > 64. Correct.

But there are 9 other callers of `raw_bits()` to audit individually.

Verdict: targeted manual audit of every `raw_bits()` caller; specifically
look for any that don't have the >64 fallback.

## Specific code paths flagged for follow-up

### F1. JIT path on wide signals (codex finding, confirmed)

`compile_edge_blocks` (simulator.rs:4364) compiles every edge block to
bytecode. JIT (when enabled via XEZIM_JIT) further lowers some bytecode
to LLVM IR. JIT's load/store bridges (`jit_load_signal`,
`jit_load_array_elem`) are u64-only. If an edge block reads a >64-bit
signal, the JIT version silently truncates and produces wrong results.

**Repro condition:** XEZIM_JIT=1 + any c910 design (which has many >64-bit
signals: AIQ entries 227-bit, VIQ 151-bit, NbaAssign 178-bit, etc.).

**Status:** the c910 reproduce command does not enable XEZIM_JIT, so this
is latent for the current memcpy investigation. **But** if a user enables
XEZIM_JIT for c910 they will get silent corruption. This deserves a
guard.

**Proposed fix (for later, not this audit):** in JIT lowering, before
emitting a block's IR, scan its inputs/outputs and reject if any signal
width exceeds 64. Fall back to interpreter path for that block.

### F2. `jit_inputs_have_xz` >64 fallback already correct

Verified. simulator.rs:13868 has the explicit width>64 walk over bits.
No change needed.

### F3. Comparator / sort u64-truncation

Category B sites (11984, 12004-08, 15424, 15441). Wide signals compared
via low 64 bits only. Currently latent for c910 (no known affected RTL).

**Proposed fix (for later):** introduce `Value::cmp_unsigned` that walks
all bits in MSB-first order. Replace the sort comparators when triggered
by a real bug.

### F4. `eval_expr().to_u64()` for arithmetic operations

Some places in simulator.rs perform arithmetic on `to_u64()` directly,
bypassing the full Value path. Spot-check needed:
- 11191 `let n = v.to_u64().unwrap_or(0);` — used for $log10/etc.
  `f64::ln` accepts u64. Safe for our purposes.
- 11214 `f32::from_bits(v.to_u64() as u32)` — bit reinterpretation.
  Fine for 32-bit floats.
- 14217-14218 `eval_expr(left).to_u64()` then arithmetic — line 14217
  context is shift count. Safe.

No further hits found that compute a wide-Value result via u64 arithmetic
where the existing Value::add/sub/mul wide path (commit 710a793) wasn't
used.

## Recommendations (priority order)

1. **JIT block reject for width > 64** (F1): adds a one-time scan during
   JIT lowering, refuses unsafe blocks. Closes the silent-corruption gap.
   Test: write a synthetic 128-bit-wide register module, run with
   `XEZIM_JIT=1`, confirm interpreter path is taken.

2. **Document `to_u64()` truncation semantics** in the Value docstring
   (`xezim-core/src/value.rs:359`). Currently the function's behavior
   for wide values is implicit. A line comment "WARNING: truncates to low
   64 bits for width > 64; use `to_u128()` or `get_bits()` if you need
   full width" prevents future misuse.

3. **`jit_load_signal` + `jit_load_array_elem` debug asserts** to catch
   wide-signal accidental use during development. Cheap to add.
   `debug_assert!(self.signal_widths[id] <= 64, "JIT cannot load signal
   wider than 64 bits");` - In release builds this is a no-op.

4. **Replace u64 sort comparators with Value-aware compare** (F3) IF a
   real test fails. Not urgent; low signal-to-noise to fix preemptively.

## What this audit DOES NOT cover (out of scope)

- The 117 `from_u64` calls in simulator.rs were not individually
  classified; spot-checks suggest they're used for narrow constants only.
- LLVM IR builder calls in jit_llvm.rs were not deep-audited; surface
  scan suggests `as u64` there is mostly metadata not value bits.
- This audit does not prove the c910 memcpy bug is or is not a 64-bit
  issue. The current working theory (post-case-stmt-retraction) is
  upstream IB buffer corruption, which may or may not involve 64-bit
  truncation.

## Next steps if user wants implementation

1. Apply recommendation #2 (docstring) — trivial, no behavior change.
2. Apply recommendation #3 (debug_assert) — no release-build behavior
   change, catches dev errors.
3. Apply recommendation #1 (JIT reject) only if c910 actually runs with
   XEZIM_JIT. Otherwise latent.
4. Recommendation #4 only on demand.
