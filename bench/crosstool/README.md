# Cross-simulator benchmarks

Five benchmarks for comparing xezim against other SystemVerilog simulators.
Distinct from `bench/`, which compares xezim against *itself* across hardware —
these compare *tools*, so everything here is plain IEEE 1800 with no
vendor-specific constructs, no tool-specific command-line forms in the source,
and no simulator-dependent behavior in any value that gets checked.

| # | Benchmark | Axis it isolates | Dominant cost |
|---|-----------|------------------|----------------|
| B1 | `b1_comb_mesh` | combinational propagation | event scheduling, continuous-assign evaluation |
| B2 | `b2_pipeline` | nonblocking-assignment throughput | edge detection, NBA region |
| B3 | `b3_memory` | array storage and indexed access | value representation, element addressing |
| B4 | `b4_oop_tb` | class/testbench runtime | allocation, queues, assoc arrays, mailbox, fork/join |
| B5 | `b5_elab_scale` | hierarchy elaboration | flattening, parameter resolution, per-instance setup |

B5 is mostly a **compile-time** benchmark — time your tool's separate elaborate
step (for xezim, `--compile`); its run phase is deliberately short.

## Why these are comparable across tools

* **No `$random`, no `randomize()`.** `$random`'s generator is
  implementation-defined and constraint solvers legitimately pick different
  legal values, so neither can appear in a checksum. All data comes from an
  explicit LFSR written in the source.
* **Every benchmark self-checks.** Each prints `BENCH <name> PASS|FAIL` based on
  structural invariants (pipeline latency, memory readback, queue/associative
  counts, hierarchy completeness) — not on a hardcoded magic number.
* **Every benchmark prints a `CHECKSUM`.** Conforming simulators must produce
  **identical** checksums. A checksum mismatch is a correctness difference, and
  it must be resolved before any timing comparison means anything.
* **Three sizes**, selected by a define: `BENCH_SMALL`, default, `BENCH_LARGE`.
  Start small to verify agreement, then scale up for timing.

## Running

```bash
# xezim (release build)
./run_crosstool.sh                 # default size
./run_crosstool.sh -s small        # verify agreement first
./run_crosstool.sh -s large        # then time

# any other simulator: give its compile and run commands.
# {SRC}, {TOP} and {DEF} are substituted; {DEF} is the +define+ for the size.
./run_crosstool.sh \
   -c '<your-compiler> -sv {DEF} {SRC}' \
   -r '<your-simulator> {TOP}'
```

Per-file invocation, if you prefer to drive the tools yourself:

| file | top module |
|------|------------|
| `b1_comb_mesh.sv` | `bench_comb` |
| `b2_pipeline.sv`  | `bench_pipe` |
| `b3_memory.sv`    | `bench_mem`  |
| `b4_oop_tb.sv`    | `bench_oop`  |
| `b5_elab_scale.sv`| `bench_elab` |

## Golden checksums (`BENCH_SMALL`)

Cross-verified between xezim and a commercial reference simulator. Any tool
printing something else for these is wrong, whatever its wall time says:

| benchmark | checksum |
|-----------|----------|
| b1_comb_mesh  | `0000005ffa263660` |
| b2_pipeline   | `000003ec5a89fc8a` |
| b3_memory     | `000026ca96f1e2d7` |
| b4_oop_tb     | `0000040d0e280e3d` |
| b5_elab_scale | `0000000000002613` |

### Golden checksums (default size)

Also cross-verified between xezim and the reference:

| benchmark | checksum |
|-----------|----------|
| b1_comb_mesh  | `000007ea656c9160` |
| b2_pipeline   | `000026ad535b416d` |
| b3_memory     | `00030e5fef027d09` |
| b5_elab_scale | `000000000000259b` |

## Interpreting results

* **Subtract the tool's startup floor, or use `-s large`.** Tools that run
  separate compile/elaborate/simulate binaries pay that cost per invocation;
  measured on one commercial simulator here,
  that floor was ~16 s per benchmark regardless of size, which completely
  swamps the small and default presets. Measure the floor once (run the small
  preset, which does almost no work) and subtract it, or scale up with
  `-s large` until the work dominates. Reporting a speedup that is really a
  process-startup difference is the classic way to produce a meaningless
  benchmark.
* Compare `WALL_S` **only between runs that both PASS with matching
  checksums**. Timing a run that failed measures nothing.
* `WORK` is the fixed unit count the benchmark performed; divide to get a rate
  (stage-evals/s, NBA-updates/s, accesses/s, packet-ops/s). Rates are the
  portable number — wall time alone depends on the size preset.
* Expect the *shape* to differ, not just the magnitude: a tool can be fast on
  B1/B2 (RTL event scheduling) and slow on B4 (testbench runtime), and that
  difference is the useful finding.

## Known result: B4 fails on xezim

As of this writing `b4_oop_tb` **FAILs on xezim** while the reference passes it.
A bounded-mailbox producer/consumer running in zero simulation time loses the
consumer's continuation, so `join` returns while a child process is still
mid-loop (produced/consumed stop short of `N_PKT`). Plain `fork`/`join` without
a mailbox is unaffected. This is a real xezim defect, left visible on purpose —
a benchmark suite that hides its own tool's failures is worthless.
