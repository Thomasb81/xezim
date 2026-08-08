# xezim performance work — August 2026

Reference workload: **XuanTie C906**, memcpy firmware, `+iterations=50`
(`simtest`-style run reconstructed from `rtlmeter`). Cross-checked on **XuanTie C910**
and on CoreMark, which is ~23x heavier in simulated time.

---

## 1. Headline result

Clean interleaved A/B on an idle machine, 2 reps, `XEZIM_NO_PARALLEL=1`:

| | before | after |
|---|---|---|
| wall, c906 memcpy x50 | 55.4 s | **32.2 s — 1.72x** |
| compile phase | 14.6 s | **3.0 s — 4.8x** |
| simulation loop | 40.4 s | 29.1 s — 1.39x |
| retired instructions | 387.7 G | **284.0 G — −26.7%** |
| executed bytecode instructions | 1,707 M | **1,276 M — −25.2%** |

Cross-design, interleaved (machine under load, so these understate):

| workload | before | after | notes |
|---|---|---|---|
| c906 cmark x2 | 1855 s | 1267 s (1.46x) | 152 M ticks |
| c910 cmark x2 | 3866 s | 2428 s (1.59x) | different design |
| c910 memcpy x50 | 274.2 s | 206.8 s (1.33x) | never tuned against |

**Every result is bit-identical to the pre-optimization binary** — same cycle counts,
same simulated finish times, byte-identical stdout.

Test suite went 1739 passed / 3 failed → **1770 passed / 0 failed / 13 ignored**
(+22 tests added by this work; the 3 prior failures were environmental and were fixed
upstream in 0.9.8).

---

## 2. What landed, and why each worked

Ordered by size of contribution.

### 2.1 Compile phase: 15.2 s → 3.3 s

`classify_one_always_block` computed `comb_sensitivity_is_faithful` **eagerly for every
always block** — two `HashSet<String>` built from `format!`-ed hierarchical names, plus
a scope inference and per-name id resolution — although it is only consulted when four
cheaper predicates have already admitted the comb path. On a CPU core almost every
block is `always @(posedge clk)`, where `all_level` is false and the result is thrown
away. Folding it into the `&&` chain as the last operand lets it short-circuit.

**11.58 s → 46.6 ms** for that predicate. 76% of the entire compile phase.

### 2.2 Redundant `Resize` deleted at compile time: −14.1% of executed bytecode

Instrumenting the VM handler showed **99.7% of `Resize` executions were no-ops** — the
register already had the target width (242.4 M of 243.1 M). `Resize` was the second most
frequent opcode.

A static width-inference pass over the emitted stream now deletes them. Unknown width
keeps the instruction; branch targets invalidate the table. `Concat` (sum of operand
widths) and `Replicate` were the decisive rules — without them only 87.6% of dynamic
resizes went, with them 98.5%. The conservative control-flow barrier blocked **zero**
eliminations, so a real dataflow merge is provably unnecessary.

Static `Resize` 89,071 → 11,799. Validated by a debug mode that keeps each deleted
instruction and asserts the width already matched: **zero assertions over 56 M
instances** across the full test suite.

### 2.3 Bytecode VM: in-place results, −9.7% instructions

Every hot arm was `vm_regs[d] = vm_regs[l].op(&vm_regs[r])` — constructing a fresh
32-byte `Value`, moving it, and dropping the destination's previous contents (a branch
on the storage discriminant plus `free` when it was `Wide`). The inline (≤64-bit) case
now writes the destination's two words in place. Each `vm_*` helper returns `None` for
any shape it does not reproduce, so the original `Value` method still handles it.

`Replicate` built a `Vec` of N clones and turned out to be the hot `concat_refs` caller.

### 2.4 `Value` hot paths: −5.3%, then −4.6%

`lto = false` means every un-annotated `pub fn` in `xezim-core` is a real cross-crate
call from the VM loop. `range_select`, `bit_select` and `resize` were split into
`#[inline]` heads with `#[inline(never)]` cold tails.

Then byte-parallel `Wide` paths: `LogicBit`'s `#[repr(u8)]` discriminant *is* the
`(xz<<1)|val` code, so `Wide` storage converts to/from packed planes 8 bits per
multiply. `concat_refs`' wide arm was measured at 1.33 M calls x ~153 operands with
**99.2% of operands one bit wide** — the cost was per-operand, not per-bit.

### 2.5 Instruction fusions

Driven by a dynamic opcode census (`--features opcode-census`, `XEZIM_OPCODE_CENSUS=1`),
not by guessing.

| fusion | occurrences | effect |
|---|---|---|
| `LoadSignalBit ; BranchIfFalse` → `BranchIfSignalFalse(.., bit)` | 25.4 M | −4.8% of stream |
| `LoadSignal ; LoadArrayElem ; NbaAssign` → `NbaAssignArrayRead` | 16.5 M | −5.4% bytecode |
| `LoadConst ; Add\|Eq\|CaseEq` → `BinOpConst` | 32.3 M | −8.0% bytecode, −2.6% wall |

The array-read triple is an RTL memory read feeding a flop — the dominant shape in a
CPU's register file and caches. Both constituent pairs reported *identical* census
counts, which is what identified it as one idiom rather than two.

### 2.6 Edge detection: −4.4%

`after_signal_write` called `raw_bits()` — whose `Wide` arm repacks a byte per bit —
*before* the guard deciding whether the result was wanted. `snap_one` never inlined
despite `#[inline]` (six parameters, one a `&mut HashMap`). The detect loop
re-materialized every table base pointer from `self` each iteration because its stores
could alias.

### 2.7 Settle: −3.9%

110 M writes from the fused arms took a `mark_dirty_id` → `dirty_list` push → rescan →
flag-clear → dep-walk round trip that is net-zero within a settle. They now trigger
dependents directly.

### 2.8 Smaller items

- **Opcode census compiled out** (−1.3%): the census flag test sat on the VM's dispatch
  critical path. Now behind `--features opcode-census`.
- **`forced_signals.is_empty()` guard**: `contains_key` hashed the id on every signal
  write even when nothing was forced.
- **`%m` scope interning** (see §4): recovered 85% of a regression that arrived with
  upstream 0.9.8.
- **`Insn` signal ids `usize` → `u32`**: instruction-neutral; landed as a prerequisite
  and for a checked narrowing choke point.

---

## 3. Correctness fixes found while profiling

- **JIT dropped the X/Z plane.** The inline-bits `LoadSignal` fast path loaded only
  `val_bits` and never wrote `xz_slots[dest]`. Registers are 4-state, so **every X/Z
  signal read back as a determinate value** — silently wrong results, not a crash. It
  wedged c906 under `XEZIM_JIT=1 XEZIM_INLINE_BITS=1`. Same trap that had already taken
  the NBA fast paths out of service; the read path was missed at the time.
- **JIT width table missing entries.** `jit.rs` keeps its *own* register-width table
  driving post-op masking, and it lacked `RangeSelectConst` and `Select`. Deleting a
  `Resize` whose width came from those would have left the JIT masking with width 0.
- **`%m` inside `always_comb` reported the wrong instance** — found during the Stage 1
  experiment, caused by comparing declared vs actual width before the wide check.

---

## 4. An upstream regression, isolated and fixed

Upstream 0.9.8's `5387b14 fix: %m names the instance in sensitivity-driven blocks`
added to `exec_bytecode`:

```rust
if self.m_block_scope != self.edge_blocks[block_idx].scope { ... }
```

A `String` compare plus an `Arc` deref plus a scattered load into a 3607-entry struct
vector, on **~189 M block fires**. Cost **+9.8 G retired instructions (+3.5%) and +9.5%
simulation time** — with every work counter bit-identical (entry_evals, edges_fired,
bytecode instructions all unchanged). Same work, more time, which is the signature of
per-operation overhead rather than a behaviour change.

Fixed by interning each edge block's scope to a dense `u32` at compile time and
comparing ids; blocks sharing an instance share an id, so consecutive fires in one scope
skip the buffer copy. **85% of the regression recovered.** This fix is a candidate to
send upstream rather than carry locally.

---

## 5. Measured and rejected

Eleven candidates. Each was plausible; each lost. Recorded so none is retried blind.

| candidate | result | mechanism |
|---|---|---|
| **SoA signal store (mirror)** | −9.4% | see below |
| **SoA signal store (authoritative)** | **+11.3%** | header tax — see below |
| **Cranelift JIT** | +33–55% wall | VM registers in stack slots; IPC 2.30 → 1.78; 28% coverage |
| **LTO** | thin +4.27%, fat +1.22% wall | `value.rs` already has ~90 `#[inline]` added *because* `lto = false` |
| **`target-cpu=native`** | +2.36% wall | retires 1.94% *fewer* instructions and is slower |
| **PGO** | −2.9% wall | only build-config winner; dropped by direction (staleness risk) |
| **Dense NBA index** | +3.3% | 140 MB array; a compact hash caches better than sparse probes |
| **serde replacement** | n/a | zero serde symbols in a default run's profile |
| **`prev_val` by edge position** | <1% | needs a 5-structure sync; failure mode is a silently missed edge |
| **Buffer-chain collapsing** | <1% ceiling | see below |
| **Two more branch fusions** | parity | codegen tax — see §6 |

### 5.1 The SoA result, and the correction that matters

The mirror form (`signal_inline_bits` beside `signal_table`) measured 9.4% slower, and
the natural reading was *"the cost is maintaining two live representations; one
authoritative store would be fine."* **That reading was wrong.**

A full migration was built: authoritative 16 B/signal store, mirror deleted, `soa.rs`
deleted, wide signals in a side table, X/Z exactness proved by an instrumented probe
over 35.1 M + 36.0 M signals and all 1770 tests with zero violations. Result:

| | c906 | c910 |
|---|---|---|
| instructions | 283.95 G → **316.15 G (+11.3%)** | +5.0% |
| wall | +9.4% | slower |
| peak RSS | **−212 MB (−8.0%)** | **−444 MB (−11.5%)** |

**The real mechanism is separating the scalar header from the bits.** A 32-byte `Value`
carries `width`/`is_signed`/`is_real` in the *same cache line* as the bits, for free. A
16-byte val/xz cell forces every access to re-fetch them from three separate 35–140 MB
arrays — 3 extra loads on `LoadSignal`, which is 16.3% of the executed stream. The
accessor layer alone, before any storage changed, already cost +21.2%: a bounds-checked
index into `signal_widths[]` can panic, so LLVM cannot elide the load.

Confirming evidence: the **small benches got ~11% faster**. With few signals the header
arrays stay cache-resident and the halved footprint wins. The loss is specific to
designs with large scattered header arrays — i.e. real ones.

Patch preserved at `STAGE1.patch` (6,647 lines, all gates green) if the memory win is
ever wanted on its own.

### 5.2 Buffer-chain collapsing

Chains exist — 5,358 buffer-like comb entries, 49.4% chained, overwhelmingly 1:1 copies
— but the ceiling is under 1%: buffer-like entries are only 12.3% of settle evaluations,
`settle_iters/settle_calls` is 1.36 (so chains cost no extra passes), and collapsing
removes dependency *depth*, not work — the intermediate signal is observable, so both
writes still happen. Settle is dominated by `FusedGate` at **57.8%** of evaluations,
95% of which genuinely change their output.

---

## 6. The fusion ceiling

Two further fusions were built and validated — `BinOpConstCaseEq;BranchIfFalse` (9.1 M,
100% of that opcode's executions) and `LoadSignal;BranchUnlessZero` (13.9 M) — capturing
**23,029,102 pairs exactly** and cutting bytecode **5.07%**.

They were **reverted**, because:

| config | retired instructions |
|---|---|
| before the patch | 284.03 G |
| variants added, **fusions disabled** | **287.98 G (+1.39%)** |
| variants added, fusions enabled | 284.08 G |

**Merely adding two arms to the 65-variant `match` costs +1.39% — for code that never
runs.** The bytecode win is exactly cancelled. Defaulting them off would be strictly
worse. Patch preserved at `branch-fusions-DEFERRED.patch`; it becomes worth landing once
dispatch is a dense table rather than a large enum match.

Per-instruction cost is now hostage to how LLVM lays out that match, and it moves by
more than an entire fusion's worth when perturbed. **Further fusion work is
self-defeating until the instruction representation changes.**

---

## 7. Remaining opportunities, ranked

Profile after all of the above (idle machine, c906 memcpy x50). IPC 2.24,
branch-miss 1.14%, cache-miss 6.4% — **instruction-bound, not memory-stalled**.

```
42.80%  exec_insns                     2.86%  after_signal_write
18.19%  settle_combinatorial_inner     2.56%  snapshot_edge_signals
12.03%  check_edges_inner              2.13%  allocator
```
Top three are 73%.

1. **Pack the header into `Insn`.** `LoadSignal(RegId, SigId)` uses 12 of the enum's 24
   bytes. Packing `width | signed<<30` removes 2 of the 3 header loads on 16.3% of
   executed opcodes; both are immutable at run time. Small change, no storage
   migration. *(`is_real` must still be loaded — the `signal_real` classifier
   under-reports for real array cells and parameter-port reals.)*
2. **Flat `ExecInsn` representation.** `u8` opcode into a dense table, pool indices
   replacing the 11 `Box`-carrying variants (6 exceed a 15-byte payload and are the
   binding constraint on 24 B). Removes the codegen coupling in §6 and makes existing
   fusions actually show up. Expected 2–5%, plus codegen stability.
3. **Typed VM registers.** 586 M register reads measured **99.826% inline**; `is_real`
   and `is_fill` occur **zero** times in a whole run. Note `vm_regs` is a single shared
   scratch `Vec` reused by every block, so 29% of stores change width — `RegMeta` must
   live on `CompiledBlock`, not a global parallel array. Expected 2–3%.
4. **24-byte signal cell** `{val, xz, width, flags}` — 842 MB vs 1124 MB (−25%) with
   *zero* extra loads. The layout that gets a footprint win without the header tax.
5. **Re-land `branch-fusions-DEFERRED.patch`** after (2).

Not promising: settle and edge detect have each had two passes and are dominated by
necessary work; the JIT needs codegen quality before coverage (raising coverage first
makes it slower).

---

## 8. How to measure this correctly

Getting this wrong produced several false conclusions before the rules were established.

- **Always `XEZIM_NO_PARALLEL=1`.** The parallel-dispatch path self-calibrates on *wall
  clock*, so two runs of an identical binary can differ ~10% in retired instructions.
- **Retired instructions for source changes** (contention-independent, ~0.1%
  reproducible); **wall clock decides codegen/layout changes**. Two cases proved this:
  the JIT retired 7.2% *fewer* instructions and was 33–55% slower; `target-cpu=native`
  retired 1.94% fewer and was 2.36% slower.
- **Same-binary A/B via an env escape hatch** (`XEZIM_FUSE_CONST=0` etc.) removes
  build-to-build noise. This is what caught the +1.39% codegen tax in §6.
- **≥4 interleaved reps**, and reverse the order on a second pass. One engineer
  concluded the JIT won on wall time; order-reversed at matched load, it lost.
- **`cargo test --release` fail-fasts** — always `--no-fail-fast`, and run each crate
  from its own directory.
- **Profile with `perf record --call-graph lbr`** — `panic = "abort"` strips unwind
  tables, so DWARF call graphs do not work.
- **The machine must be genuinely idle** for wall numbers. Contended runs understated
  the headline by ~45%.

---

## 9. Regression gates

Any change must reproduce all of these.

| gate | expected |
|---|---|
| c906 memcpy x50 | `cost 727`, finish `6477650` |
| c906 cmark x2 | `714196` cycles, finish `152197450`, CoreMark `1.400176` |
| c910 memcpy x50 | `216` cycles, finish `2282050` |
| c910 cmark x2 | `158034` cycles, finish `34985250`, CoreMark `6.327752` |
| `b2_vm_dispatch` | `cycles=200000` |
| `b3_mem_sweep_20` | `cycles=100000` |
| `b2b_vm_branchy` | `cycles=50000` |
| tests (`--no-fail-fast`, both crates) | **1770 passed, 0 failed, 13 ignored** |
| feature builds | `--features jit`, `--features opcode-census` |

`checksum=x` on the benches is the expected value, not a failure — `%0d` renders as `x`
when the value has unknown bits.

**The strongest and cheapest check is a full stdout `diff` against the previous binary.
Byte-identical is the bar**, and it caught more than any golden number alone.

Also run at least one gate *without* `XEZIM_NO_PARALLEL=1` — that is the only way
`exec_comb_block_isolated` and the partitioning analyses get exercised.

---

## 10. Hazards specific to this codebase

1. **~25 analysis sites match `Insn` with a catch-all `_ =>`** to extract signal ids. A
   new variant compiles fine while being silently ignored there, producing wrong event
   gating rather than an error. This is how the JIT's `is_supported` list rotted to 28%
   coverage with no build error. **Mitigation: change the ARITY of an existing variant
   instead of adding one** — every pattern then fails to compile and the compiler hands
   you the complete site list. Used successfully three times.
2. **`build_event_measure_state`** drives event-edge gating. A missed read means a flop
   is skipped when its input changed — silent wrong answer.
3. **Branch polarity.** `BranchUnlessZero` jumps on **true or X**;
   `BranchIfSignalFalse` jumps on **false or X**. Opposite, agreeing only on X.
   Conflating them inverts a branch and still terminates with plausible output.
4. **Width truncation.** `Value::MAX_WIDTH` is `1<<20`, so width does not fit a `u16`.
5. **`jit.rs` keeps its own register-width table**, separate from the bytecode
   compiler's. Width-inference changes must update both.
6. **`exec_comb_block_isolated`'s match is now exhaustive** — do not reintroduce a `_`
   arm. It is the safety net for any future port of the exec loops.
7. **`$root.`-prefixed hierarchical references in an event control are mis-simulated.**
   rtlmeter's `always @(posedge $root.<clk>) ++cycles` counts 1 instead of 39,567. Use
   the firmware's own cycle count, not `_rtlmeter_cycles.txt`. Not fixed.

---

## 11. Reproducing the workloads

RTL is not in these repos. `simtest/xuantie_c906/work/c906.fl` points at a path that
does not exist; clone `https://github.com/verilator/rtlmeter` instead. Build the file
list from `descriptor.yaml`'s `compile.verilogSourceFiles` and append
`rtlmeter/rtl/__rtlmeter_utils.sv`. Each run needs its own directory — the testbench
`$readmemh`s `inst.pat`/`data.pat` from the CWD.

```bash
XEZIM_NO_PARALLEL=1 XEZIM_INIT_ZERO=1 xezim --simulate --max-time 100000000 \
  -s tb "-D__RTLMETER_MAIN_CLOCK=tb.clk" \
  -I <design>/src -I rtlmeter/rtl -f <design>.fl +iterations=50
```

`__RTLMETER_MAIN_CLOCK` must be defined or `__rtlmeter_utils.sv` is a syntax error.
`XEZIM_INIT_ZERO=1` is required for cmark.

**Timescale is `1ns/100ps`, so reported "finished at time" is in 100 ps ticks** —
divide by 10 for ns. Getting this wrong makes xezim look 16x slower than it is.

### Comparison with Verilator

Verilator runs the same c906 memcpy in 0.89 s. But the two are **not simulating the same
work**: Verilator finishes at 396 us / 39,567 clock cycles, xezim at 647.8 us
(~64,776 cycles), and the firmware reports 727 vs 368 CPU cycles for the copy. xezim
runs **~1.64x more clock cycles for the same firmware**, so any speed ratio must be
normalized for that — roughly 27x per simulated cycle rather than the ~44x the raw wall
times suggest. The cause of that divergence was not investigated; `XEZIM_INIT_ZERO=1`
forcing X→0 everywhere is a plausible suspect.

---

## Combinational cone merging — measured, rejected as a large win (Aug 2026)

The proposal: compile chains of `CombEntry` (`c = a&b; d = c^x; y = d|z`) into one
kernel invocation, removing the per-entry scheduler work, dirty-queue traffic, entry
lookup and dispatch at every hop. Estimated beforehand at 5–15% (small RTL) and
15–40% (large DUT-heavy).

**The tree already contains the analyzer for this.** `simulator.rs:19146`, opt-in via
`XEZIM_CONE=1`, whose header states the design verbatim: "entry A writes signal s, and
B is the ONLY reader of s, and s has no other writer — then A+B can become one entry".
It was extended here to report *merge economics*, because chain-shape alone does not
tell you whether merging saves work.

### Why chain-shape alone is not the answer

A merged kernel fires when **any** member's inputs change and then recomputes **every**
member. So its best-case work is `max(member evals) * len`, against `sum(member evals)`
unmerged. Members that fire equally often give a ratio of 1.0 (merging is pure win);
skewed members give >1.0, and that extra compute has to be paid for out of the saved
dispatch.

| | entries | chain-members (static) | eval-weighted | blanket-merge compute |
|---|---|---|---|---|
| c906 memcpy | 52,124 | 39.8% | **43.0%** | **+32.8%** |
| c910 memcpy | 367,320 | 29.8% | **25.0%** | **+44.2%** |

Blanket merging is a wash or a loss: on c906 it removes 50.4% of in-chain dispatches
(37.06M → 18.38M) but adds 32.8% compute (37.06M → 49.22M evaluations).

**The size effect is the opposite of the intuition.** The larger, more DUT-heavy design
has less than half the opportunity, and its hot entries are *less* chain-shaped than its
static share (25.0% vs 29.8%) while c906's are *more* (43.0% vs 39.8%).

### The viable form is selective, and its ceiling is ~1%

77% of c906 chains (6,406 of 8,313) recompute at <1.1x — those are nearly free. Folding
only those:

| threshold | c906 dispatches removed | c906 extra compute | c910 removed | c910 extra |
|---|---|---|---|---|
| **<1.1x** | **14.9% of all evals** | **+0.1%** | **5.7%** | **+0.0%** |
| <1.5x | 17.5% | +1.9% | 6.8% | +0.7% |
| <2x | 18.6% | +6.4% | 7.3% | +4.8% |
| all | 19.0% | +12.3% | 7.7% | +7.9% |

Past <1.1x the trade turns bad fast — c906 <2x buys +3.7pp of dispatch for +6.3pp of
compute.

`settle_combinatorial_inner` is 18.19% of the run at 222 retired instructions per entry
evaluation. So even if dispatch were *100%* of that cost, the selective merge ceiling is
`14.9% x 18.19%` = **2.7% on c906 and 1.0% on c910**. Dispatch is realistically ~60 of
those 222 instructions (~50–70 of overhead against ~20–25 of actual gate algebra for a
`Bin2`), which puts the expected win at **~0.7% / ~0.3%** — an order of magnitude below
the estimate, for a transform that must also keep interior nets observable to VCD,
`force` and VPI, preserve ordering, and exclude NBA-bearing and unresolved-read entries.

### Independent corroboration

`XEZIM_BSP_SETTLE=1` already implements the coarse form of the same idea — evaluate by
static topological level instead of by dirty queue. Measured on c906 memcpy x50:
**354.4G retired vs 284.2G, +24.7%**, with `cost=727` (correct). Over-evaluation is
precisely what makes group evaluation lose here, and it is the same force that caps
cone merging.

### What the analysis *did* produce

The per-dispatch constant is real and large. Cone merging just happens to remove it from
only 15% (c906) / 6% (c910) of evaluations. Two changes attack the identical cost across
**100%** of them:

1. **Re-land the `entries[]` prefetch.** `simulator.rs:32905` records that a single-stage
   `_mm_prefetch` on `entries[cur_list[cur_pos+8]]` gave **c906 cmark a 31% wall-time
   win**, reverted because c910 began hanging at iters=200040 with it active "despite the
   prefetch being a semantic no-op". A semantic no-op that changes termination indicates a
   **latent bug exposed by timing** — worth hunting on correctness grounds alone. Gate any
   re-land on c910 cmark t=1 k=0 *and* t=4 k=4.
2. **`settle_triggered: Vec<bool>` → bitset.** One byte per entry = 367 KB on c910,
   randomly probed once per dispatch and once per dependent inside `trigger_deps!`
   (119.1M dirty-propagation round trips per c906 run, 93% of them from the fused arms).
   A bitset is 8x denser at 46 KB — L2-resident — and is touched by every evaluation.

The `XEZIM_CONE` economics reporting added for this investigation is diagnostic-only and
opt-in; default-path instructions and stdout are unchanged (284.35G vs 284.20/284.22G
baseline, byte-identical output, 1742 tests passing).
