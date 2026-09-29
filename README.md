# xezim — SystemVerilog Simulator (Rust)

**xezim** is an **extensible, AI-native SystemVerilog simulator written in Rust** — built so new language features and analyses can be added one verified step at a time, with AI agents as first-class contributors to the codebase.

> `xezim` was previously developed under the name `sisSIM`. The binary, library, and compiled-artifact magic were renamed in place; behavior is unchanged.

This project explores whether modern tools and AI can dramatically reduce the complexity of building core EDA infrastructure such as simulators.

The simulator parses SystemVerilog source code, builds an internal representation, and executes simulations for combinational and sequential logic.

---

# Motivation

Traditional EDA tools require very large engineering teams and many years of development.

This project explores a key question:

> Can a small team — or even a single engineer with AI assistance — build core EDA tools such as a SystemVerilog simulator?

The simulator is being developed incrementally, starting from simple combinational logic and gradually adding more SystemVerilog features.

---

# Features

Current capabilities include:

* IEEE 1800-2023 grammar by default (`--sv2017` opts back to the earlier edition)
* Event-driven simulation of RTL and gate-level netlists — continuous
  assignments, procedural blocks and the IEEE 1800 scheduling regions, UDPs and
  drive strengths, specify-block delays and timing checks, and SDF
  back-annotation (`--sdf`)
* Classes, constrained randomization, covergroups and concurrent assertions
  (SVA) — the base the UVM support below runs on
* Waveform / trace dumps (**`--wave`**, off by default) — VCD
  (`$dumpfile`/`$dumpvars`; IEEE 1800-2017 §21.7, and matches Verilator/Icarus
  in GTKWave), **FST** (`--fst`, GTKWave's binary format, written on a
  dedicated writer thread with scope filtering), and XTrace v1.0 (`--xtrace`,
  optional zstd compression + scope filtering). All three are cross-checked
  against each other by decoding them, not by file size. Dumping is opt-in at
  model-compile time because it is not free — an active dump forces loops that
  would otherwise compile onto the AST path and builds a per-signal trace
  table — so `$dumpvars` needs `--wave` and warns once without it. `--fst` and
  `--xtrace` are explicit dump requests and imply `--wave`.
* **UVM run-phase execution** (Accellera **1800.2-2017 and 1800.2-2020.3.1**, with
  `-DUVM_NO_DPI`) — a real UVM testbench runs end-to-end: build → connect → topology →
  `run_phase` stimulus → sequencer↔driver TLM handshake → packet collection →
  objection-driven termination → report summary. The reference testbench
  (GettingVerilatorStartedWithUVM) reaches exact Verilator parity on the 2017
  library and runs green on 2020.3.1, and 32/35 UVM 1800.2-2017 example
  testbenches pass. The mbits-mirafra AVIP base tests for axi4, apb, i3c, spi
  and axi4Lite print the same UVM messages as a commercial reference
  simulator. Multiple top
  modules (`-s hdl_top -s hvl_top`) and virtual-interface `config_db` are supported.
  See [docs/uvm-guide.md](docs/uvm-guide.md).
* **UVM's DPI-C library, built in** — compile UVM without `-DUVM_NO_DPI` and its
  regex matching (config_db / resource_db wildcards and `/regex/` scopes,
  `+uvm_set_*` plusargs, factory overrides by path), command-line processing and
  `uvm_hdl_*` backdoor access (read / deposit / force / release by full path,
  including bit- and part-selects, memory words and packed-struct members) run
  natively, with the C code's semantics and `UVM/DPI/*` error reports. A
  `--dpi-lib` library defining the same symbols takes precedence. See
  [docs/uvm-guide.md](docs/uvm-guide.md#uvms-dpi-c-library).
* UVM 1.2 runtime support, also demonstrated by running the `riscv-dv` instruction
  generator end-to-end (random RV32IMC programs that assemble cleanly with
  `riscv64-unknown-elf-as -march=rv32imc_zicsr_zifencei`)
* **Functional and assertion coverage**, always on — covergroups (explicit,
  automatic, array, wildcard and transition bins, `ignore_bins`/`illegal_bins`,
  crosses, `iff` guards, `at_least`/`weight`/`auto_bin_max`/`merge_instances`),
  the `get_coverage()`/`get_inst_coverage()`/`$get_coverage()` queries, and
  pass/fail counts for `cover property` and the assertions. A run with any of
  them writes the results to `xezim_cov.json` (`XEZIM_COV_DB=<path>` to move
  it). See [docs/coverage-guide.md](docs/coverage-guide.md).
* **Code coverage**, off by default — statement, branch and toggle counts with
  `--code-coverage` (or `+cover`), per instance and per design unit, in the
  same `xezim_cov.json`. Instrumented at compile time, so a run without it pays
  nothing. See [docs/coverage-guide.md](docs/coverage-guide.md#code-coverage).
* Event-driven edge gating (`XEZIM_EVENT_EDGE=1`) — opt-in skip of clocked
  flop fires whose data inputs haven't changed; 1.13-1.30× wall on the C910 /
  C906 hello / memcpy / cmark benchmarks, correct-by-construction
* **DPI-C loading** via `--dpi-lib <path>` — load shared libraries of
  `import "DPI-C"` implementations written in C or C++ (e.g. an ISS shim, a
  custom HDL-backdoor force/release layer, or your own UVM extensions). The
  repo ships minimal `svdpi.h` and `vpi_user.h` so DPI code compiles without a
  vendor install. See [docs/dpi-guide.md](docs/dpi-guide.md).
* **Power intent (IEEE 1801 UPF)** via `--upf <file>` and `--upf-top <instance>` —
  supply nets and power switches, powered-down corruption to `x`, isolation
  and retention, with the supplies driven from the testbench through the
  standard `UPF` package. See [docs/upf-guide.md](docs/upf-guide.md).
* **Event-control `iff` guards** (LRM §9.4.2.3) — `@(posedge clk iff rst_n)`
  is honored in both procedural `@` waits and edge-sensitive `always` blocks:
  the process resumes only on an edge where the guard holds.
* **Deferred immediate assertions** (LRM §16.4) — `assert #0` / `assert final`
  evaluate where they run, but their action block runs only when the report
  matures at the end of the time slot; a report is dropped if its process
  resumes first, and one still pending at `$finish` prints a note instead of
  running its action.
* **User-defined nettypes with resolution functions** (LRM §6.6.7) —
  `nettype T wire_t with resolver;` including Z-skip and built-in resolution.
* **Per-module timescales** (LRM §3.14, §20.3, §21.3.5) — `$time`/`$realtime`
  scale to the calling module's time unit; `timeunit`/`timeprecision`
  declarations scale delays; `$timeformat`/`%t` and `$printtimescale` are
  honored; precision down to `fs`. Modules without a source-level timescale can
  be assigned one from the CLI (see
  [`--module-timescale`](#module-timescale-extension)).
* **VPI loading** via `--vpi-lib <path>` (`-m`) — classic VPI modules run their
  `vlog_startup_routines`: system-task/function registration (`vpi_register_systf`)
  and design iteration (`vpi_iterate`/`vpi_scan`, handle/property access).
* **cocotb** — Python testbenches run against xezim through a runner backend
  (`contrib/cocotb/xezim_runner.py`) on top of the VPI layer, including timed and
  synchronous callbacks.
* **Native compilation** (`--features jit`) — hot bytecode compiles to machine
  code, either through the in-process JIT (`XEZIM_JIT=1`) or the AOT backend
  (`XEZIM_JIT=1 XEZIM_AOT=1`), which emits Rust for eligible combinational
  entries, edge blocks, and process FSMs, builds it with `rustc`, and caches
  the resulting library across runs. See [below](#native-compilation).
* **`bind` by instance path** (§23.11) — `bind top.u_dut.u_sub target_tb u_tb();`
  and the colon form bind only the named instances, with upward references from
  the bound module resolving against the instance they were bound into.

### Non-standard extensions

These are **not** part of IEEE 1800 — they are de-facto extensions of commercial
simulators, supported for compatibility with existing gate-level and testbench
flows. Portable code should not rely on them.

* **`$deposit(target, value)`** — sets `target` to `value` immediately *without*
  installing a persistent driver: the value holds until the next driver
  transaction overwrites it (on an undriven net it simply sticks). This is a
  Verilog-XL/VCS system task, **not** in the LRM. xezim matches the vendor
  semantics — a variable keeps the deposited value, and a real driver on a net
  overrides a deposit on its next update.
* Gate-level-simulation CLI flags — `+nospecify`, `+notimingcheck`,
  `+no_notifier`/`+no_tchk_msg`, `+delay_mode_zero`/`+delay_mode_unit`,
  `+mindelays`/`+typdelays`/`+maxdelays`, and the `-v`/`-y`/`+libext+` library
  flags — mirror the commercial spellings.

---

# Performance

Whole-run wall-clock time on one machine (Intel Core i7-9800X, 6 cores,
Linux): xezim 0.11 as a plain release build, against a commercial reference
simulator in its optimized mode (no debug visibility). Both simulators produce
the same results on every row.

| Workload | xezim 0.11 | Reference simulator |
|---|---|---|
| XuanTie C906 SoC, CoreMark ×1 (295,294 cycles) | 91 s | 82 s, 44 s of it simulating |
| XuanTie C910 dual-core SoC, memcpy ×200, cold start | 118 s, about 18 s of it compiling | 136 s, 97 s of it simulating |
| AXI4 AVIP, UVM base test | 5.8 s | 97 s, 51 s of it simulating |
| Peak memory, C906 CoreMark | 2.6 GB (0.9 GB with `XEZIM_PACKED_MEM=1`) | 57 MB |

What the table shows:

* **xezim starts fast.** It compiles and elaborates the 468-file C910 in
  about 18 s, and a short UVM test finishes long before the reference has
  started simulating. Test suites made of many short runs favour xezim.
* **On long runs the reference's kernel is faster.** Its simulation phase is
  about 2× faster on the C906. The gap is widest on UVM throughput: a sequence
  item costs xezim about 9 M host instructions against about 120 K for the
  reference, so long UVM runs still favour the reference. 0.11 cut xezim's
  cost per item by about 60%, and this is where the current work goes.
* **Memory is the other gap.** Most of the C906 figure is its large on-chip
  RAMs; `XEZIM_PACKED_MEM=1` stores byte-wide memories compactly at the same
  speed.

To get the most out of a build, use the [profile-guided
build](#profile-guided-build-recommended-for-release) (up to 14.5% fewer
instructions when trained on your own workload) and keep the [warm design
cache](#warm-design-cache) on. [Native compilation](#native-compilation) pays
on designs with few, very hot blocks (Ibex CoreMark −23%) and is a net loss on
large SoCs, so measure it on yours.

# Conformance

On the [sv-tests](https://github.com/chipsalliance/sv-tests) suite, xezim
0.11.0 passes 4,722 of 4,770 tests (99.0%). Its own regression suite is
described under [Test Suite](#test-suite).

# Release notes

Per-release change lists and earlier workload measurements live in
[NOTES.md](NOTES.md).

# Development workflow

How the repository is laid out, how to build it (including against a local
xezim-core checkout), and how to run the test suites. New contributors are
welcome; see [Contributors](#contributors) for the people behind the
project.

## Project Structure

xezim is split across two repos; this repo depends on `xezim-core` as a **git
dependency** (Cargo clones it automatically — no submodule, no manual checkout):

```
xezim-core (git dep) — shared library: parser, elaboration, value, SDF, VCD sink
./                   — bytecode interpreter + simulator (this repo, binary: xezim)
```

This repo:

```
.
├── src/
│   ├── compiler/
│   │   ├── simulator.rs   — event-driven simulator + bytecode VM
│   │   ├── bytecode.rs    — bytecode compiler for cont_assigns and always blocks
│   │   └── mod.rs         — re-exports value/elaborate/sdf from xezim-core
│   ├── lib.rs             — wraps xezim_core::parse_and_elaborate_multi + Simulator
│   └── main.rs            — CLI entry point (binary: xezim)
├── tests/                 — Rust integration tests + SV compliance suite
├── examples/
└── Cargo.toml             — depends on xezim-core (git dependency, fetched by cargo)
```

### Components

**Parser & elaboration** — live in `xezim-core`; consumed by both `xezim` and `xezim-b`.

**Simulator** — event-driven VM over a bytecode lowering of cont_assigns and always blocks.

---

## Build

Install Rust: https://www.rust-lang.org/tools/install

**If you only want to use xezim, there is nothing else to clone** — `xezim-core`
is a git dependency, and `cargo build` pulls it automatically:

```bash
git clone git@github.com:<you>/xezim.git
cd xezim
cargo build              # debug
cargo build --release    # optimized
./scripts/build-pgo.sh   # optimized + profile-guided (recommended for release)
```

The release binary is produced at `target/release/xezim`; the profile-guided
one at `pgo-target/release/xezim`.

### Profile-guided build (recommended for release)

`./scripts/build-pgo.sh` instruments the release build, trains it and
rebuilds with the profile. It is the build to ship or benchmark with.

Run **without arguments** it trains on a bundled set — the `tests/perf`
shape designs, the `scripts/pgo-train` designs and the `xezim-bench`
workloads — which takes a few minutes on top of two release builds. Measured
against a plain release build of the same sources (interleaved, same
machine, host instructions): a C906 SoC CoreMark −0.4% to −0.7%, a C910 SoC
CoreMark −0.3% to −0.7%, and a loop-heavy DRAM-model stress −12%. Output is
bit-exact (both SoC gates and the UVM AVIP suite unchanged). The bundled
trainer covers the executors and the scheduler; what it cannot know is
*your* design's hot mix, so the SoC gain is modest.

Run **with a training command** (`./scripts/build-pgo.sh xezim --simulate …`,
the binary path is substituted) it trains on that run instead. A profile
trained on the workload itself is worth far more — the C906 memcpy
benchmark:

| | instructions | wall |
|---|---|---|
| release | 176.92 B | 51.5 s |
| **PGO, trained on the run** | **151.31 B (−14.5%)** | **44.0 s (−14.6%)** |

— and it generalizes: a C906-trained profile gave Ibex −11.1% instructions
against −10.6% for an Ibex-trained one. Budget for the instrumented run,
though: the instrumented binary is far slower than release, so train on a
short run (one benchmark iteration, a few hundred thousand cycles), not the
full-length one. An unrepresentative trainer can *deoptimize* the paths you
care about.

Two more measured facts: **the wall-clock gain depends on the design being
instruction-bound** (Ibex CoreMark loses ~11% of its instructions but its
wall time does not move — its host bottleneck is memory), and **BOLT on top
of a PGO build is net negative**; PGO alone wins.

### Modifying xezim-core

`xezim-core` (parser + elaboration) is a separate repo, consumed as a git
dependency **pinned to the exact revision this xezim revision was tested
against** (see `rev = ...` in `Cargo.toml`). A bare clone therefore always
builds the verified pair — never an untested newer core — and a release tag
of xezim pairs with the core revision it shipped with. The pin is bumped in
the same commit that starts depending on new core behavior.

**Working on core?** Clone it next to (or inside) this repo and switch the
build to it — after this, plain `cargo build` uses your checkout directly,
with **no network fetch**:

```bash
git clone git@github.com:aionhw/xezim-core.git ../xezim-core
./scripts/use-local-core.sh        # detects ./xezim-core or ../xezim-core
cargo build --release              # builds against the local checkout
```

The script writes a git-ignored `.cargo/config.toml` with a `[patch]` that
overrides the pinned dependency; `./scripts/use-local-core.sh --remove`
returns to the pin. For a one-off invocation without persistent state,
`./scripts/cargo-local.sh build --release` applies the same patch for a
single command when `../xezim-core` exists.

`cargo tree -p xezim-core` shows which copy is in use (a path in parentheses
means your local checkout is active).


---

## Test Suite

The suite has **3,152 tests**: 3,110 integration tests in ten suites plus 42
unit tests (as of 0.11; 9 more are marked `#[ignore]`). Each suite is one test
binary, `tests/<suite>.rs`, with its cases in `tests/<suite>/`:

| Suite | Tests | Covers |
|---|---:|---|
| `classes` | 460 | classes, UVM, randomization, covergroups |
| `collections` | 155 | queues, dynamic and associative arrays, array methods |
| `gates` | 122 | gate primitives, UDPs, drive strengths, waveform dumps |
| `hierarchy` | 246 | instances, ports, interfaces, binds, hierarchical references |
| `misc` | 1,009 | CLI, lint, elaboration, assignments and other cases |
| `perf` | 35 | deterministic work counters that catch performance regressions |
| `scheduling` | 432 | event regions, sensitivity, timing, assertions |
| `strings` | 152 | strings, formatting, DPI |
| `types` | 495 | data types, widths, selects, operators |
| `upf` | 4 | power intent (`--upf`) |

The xezim-core repo has its own 144 tests for the parser and elaboration.

CI runs the suite twice: in a default build (`cargo test`) and in a build with
the JIT compiled in (`cargo test --features jit`). JIT execution itself is
switched on at run time with `XEZIM_JIT=1`. A large share are differential
tests whose expected values were measured on a commercial reference simulator;
their doc comments cite the LRM section and the measured behavior.

**Credit:**
All `pr*.v` tests were taken from the **Icarus Verilog test suite**.

These tests help verify correctness against real-world Verilog/SystemVerilog edge cases.

### UVM tests

The UVM integration tests (`tests/classes/uvm_integration_tests.rs`) run against
the real Accellera UVM library from https://github.com/nitronis/UVM — one repo
carrying the 1.1d, 1.2, 1800.2-2017 and 1800.2-2020 releases as subdirectories.
No manual setup is needed: `cargo build` clones it into `target/uvm-checkout`
when no checkout is found (and the tests clone on demand as a fallback). To use
an existing checkout instead, set `XEZIM_UVM_DIR` to its root or clone it as a
`../UVM` sibling of this repo.

---

## Contributing

Bug reports and pull requests are welcome. What makes them quick to land:

* **Report bugs with a small self-checking testcase** — a module that prints
  `PASS`/`FAIL` per check, plus the output you expected and where it comes from
  (the IEEE 1800 section, or another simulator's result).
* **Base the pull request on current `main`.** The code moves quickly; a
  branch several releases behind usually conflicts. Rebase before opening or
  updating it.
* **Keep formatting separate.** Run `cargo fmt` on your change, but don't mix
  a reformat of untouched code into a fix.
* **Add a regression test for each fix** under `tests/<group>/`, registered
  in `tests/<group>.rs`, that fails without the fix. Expected values should
  follow the LRM; cite the section in a comment. When you checked them against
  another simulator, say "the reference simulator" rather than naming a
  product.
* **Run the full suite** (`cargo test --release`) and include the result.
* **Mind the hot paths.** The simulator, bytecode and `Value` code run billions
  of times per simulation; for changes there, include an instruction count
  (`perf stat -e instructions`) before and after on a representative design,
  with identical output.
* **Fix the root cause**, not the one testcase, and keep each pull request to
  one topic.

# Run

Run a simple example via cargo:

```bash
cargo run --release -- examples/test.sv
```

Or invoke the binary directly:

```bash
./target/release/xezim <source_files> [+plusargs] [options]
```

A run prints the design's own output (`$display`, UVM messages, assertion
failures), warnings and errors, and one closing line —
`Simulation finished at time N`, with ` ($finish called)` when the design
finished itself. The version banner, `[PHASE]` timings and engine counters are
behind `--verbose` (or `--profile`). Parse, preprocessor and elaboration errors
name the original file, line and column — the `include`d file a line came
from, and the macro invocation for text a macro produced — and quote the line:

```text
In file included from top.sv:2:
body.svh:4:10: error: expected expression, found Semicolon ';'
    4 |   x = 3 +;
      |          ^
```

Common options:

| Option | Purpose |
|---|---|
| `-D<MACRO>[=val]` | Define a preprocessor macro |
| `-I<dir>` | Add an include directory |
| `--simulate` | Run the simulation (vs `--parse` / `--compile` / `--preprocess`) |
| `-s <module>` | Select a top-level module. Repeat for multiple roots (e.g. `-s hdl_top -s hvl_top`); each is a root of its own in `%m`, messages, `$root` paths and waveform scopes, as when several tops are found automatically. A bare module name that is not a file does the same (see [below](#command-lines-from-other-simulators)) |
| `--dpi-lib <path>` | Load a DPI-C shared library (`.so`/`.dylib`/`.dll`). Repeatable. See [docs/dpi-guide.md](docs/dpi-guide.md). |
| `--vpi-lib <path>` (`-m`) | Load a VPI module and run its `vlog_startup_routines` (system-task registration, design walk). Repeatable. |
| `--module-timescale [mods=]<unit>/<prec>` | Assign a timescale to modules with no explicit source-level one. See [below](#module-timescale-extension). Repeatable. |
| `--dump-timescales` | Print every module's resolved timescale before the run (no source `$printtimescale` needed); modules with no `` `timescale `` are flagged. See [below](#module-timescale-extension). |
| `--max-time <N>[ps\|ns\|us\|ms\|s]` | Stop simulation after `N` of simulated time (default `100ms`) — **nanoseconds** when no unit is given. The cap is resolved to whole nanoseconds (a sub-ns value rounds to the nearest one; below half a nanosecond is rejected) and then converted to the design's tick, so the same `--max-time` covers the same simulated time whatever the precision |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | Seed the RNG for a reproducible run (same seed ⇒ byte-identical output; affects e.g. the number of packets a random UVM test collects) |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate SDF delays (IOPATH/INTERCONNECT) and TIMINGCHECK limits |
| `--sim-debug` | Print `[DEBUG]` / `[OPT]` diagnostics (`--sim_debug` still accepted); implies `--verbose`'s engine lines |
| `--verbose` | Internal engine lines, off by default: the version banner, `[PHASE]` timings, the end-of-run engine counters (`[PROF]`/`[FUSE]`/`[EVENT-EDGE]`/`[COV]`), compile-time notes such as `[EDGE-MERGE]` and `[CACHE]` hits, and `--compile`'s design summary; plus per-file compile progress (each file as it is parsed, and the modules/blocks it contributed). Same as `XEZIM_VERBOSE=1` |
| `--dump-files-list` | Print the fully resolved file list after `-f` expansion, then exit — confirms *which* sources a build actually reads |
| `--dump-merged-sv <file>` | Write the sources as one preprocessed, self-contained `.sv`. With `-s <top>`, keeps only the files that top needs. See [below](#reducing-a-multi-file-build) |
| `--artifact-compression <none\|1-22>` | Compression level for the `-o` compiled artifact (`none` writes it raw) |
| `--cache-dir <dir>` | Select the automatic elaborated-design cache directory |
| `--no-cache` | Disable the automatic elaborated-design cache |
| `-l`, `--log <file>` | Redirect all stdout/stderr — including DPI/VPI C output — to a log file |
| `-v <file>` | Library file: modules compiled only to resolve unresolved instantiations |
| `-y <dir>` | Library directory: `<module>.<ext>` loaded on demand |
| `+libext+<ext>+…` | Extension list for `-y` search (replaces the default `.v`/`.sv`/`.V`) |
| `+nospecify` | Suppress specify-block path delays and timing checks — zero-delay gate simulation (`-nospecify` also accepted) |
| `+notimingcheck` | Disable the specify-block timing checks (`$setup`, `$hold`, `$width`, …; also `+notimingchecks`/`-notimingchecks`). A violation otherwise prints one `** Error:` line (counted by `--error-exit`) and toggles the check's notifier |
| `+no_notifier` / `+no_tchk_msg` | Report timing violations without toggling notifiers / toggle notifiers without reporting |
| `--wave` | Compile the model with waveform support, enabling `$dumpfile`/`$dumpvars` (off by default; `--fst`/`--xtrace` imply it) |
| `--fst <file>` | Emit an FST (GTKWave binary) waveform dump |
| `--fst-scope <hier>` | Restrict the FST dump to signals under `<hier>` (repeatable) |
| `--xtrace <file>` | Emit an XTrace v1.0 dump (`.zst`/`.zstd` ⇒ zstd-compressed) |
| `--xtrace-scope <hier>` | Restrict the XTrace dump to signals under `<hier>` (repeatable) |
| `--relax-implicit-static` | Accept `int x = ...;` inside a static task/function (§6.21) with a warning instead of an error — for vendor sources you cannot edit |
| `--error-exit` | Exit nonzero if any `$error` was reported (`$fatal` always does) |
| `--code-coverage[=<kinds>]` | Collect code coverage: `stmt`, `branch`, `toggle` (comma-separated) or `all` (the default). Results go to `xezim_cov.json` next to the functional coverage; `--verbose` adds a per-instance summary. See [docs/coverage-guide.md](docs/coverage-guide.md#code-coverage) |
| `--code-coverage-scope=<path>[,<path>...]` | Only cover these instance subtrees (`tb.dut`) and packages (repeatable) |
| `--profile` | Print the `[PROF]` end-of-run profile report (edge-block, settle and timing counters) together with the `--verbose` engine lines. Same as `XEZIM_PROFILE_REPORT=1`. Adds overhead |

Selected env knobs (off by default unless noted):

| Env var | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=1` | Skip gateable clocked flop fires whose data is unchanged (1.13-1.30× wall on c910/c906) |
| `XEZIM_JIT=1` | Compile bytecode blocks to machine code in-process (needs a `--features jit` build) |
| `XEZIM_AOT=1` | Compile eligible blocks to native code via generated Rust + `rustc` instead of cranelift. **Requires `XEZIM_JIT=1` as well** — on its own it is a no-op. Needs `--features jit`. See [below](#native-compilation) |
| `XEZIM_AOT_OPT=0..3` | `rustc` optimization level for the generated crate (default 2) |
| `XEZIM_PROC_FSM=1` | Compile blocking `always` bodies into bytecode state machines with wait instructions |
| `XEZIM_NO_NATIVE_CACHE=1` | Disable the persistent native-library cache (`~/.cache/xezim/native`) |
| `XEZIM_REGIONS=1` | Fuse dependency-connected compiled combinational entries into region blocks (experimental; currently net-negative on the benchmark set) |
| `XEZIM_STUCK_CLOCK=1` | Flag a process parked on a clock/reset that never changes while the design keeps churning edges (`abort` variant for CI) |
| `XEZIM_PACKED_MEM=1` | Store large byte-wide memories in a compact arena (C906: 2.6 GB → 0.9 GB peak memory, same speed) |
| `XEZIM_INIT_ZERO=1` | Coerce X-initialized signals/arrays to 0 (required for some C910/C906 workloads, e.g. cmark) |
| `XEZIM_PROGRESS=N` | Emit a `[PROGRESS]` line every N wall-seconds (sim_time, iters, edges_fired, nba_q) |
| `XEZIM_CACHE_DIR=<dir>` | Override the elaborated-design cache directory |
| `XEZIM_NO_CACHE=1` | Disable the automatic elaborated-design cache |
| `XEZIM_COMPILE_PHASES=1` | Report detailed simulator compilation phase timings |
| `XEZIM_ALLOW_IMPLICIT_STATIC=1` | Same as `--relax-implicit-static` |
| `XEZIM_PROFILE_REPORT=1` | Same as `--profile` |
| `XEZIM_VERBOSE=1` | Same as `--verbose` (for scripts that grep `[PHASE]`/`[PROF]` lines without adding a flag) |
| `XEZIM_CODE_COVERAGE=<kinds>` | Same as `--code-coverage=<kinds>`; the flag wins |
| `XEZIM_MAX_INST_DEPTH=N` | Instantiation-depth cap (default 200) — turns unbounded recursive instantiation into a clean error instead of memory exhaustion |
| `XEZIM_STACK_MB=N` | Stack size of the simulation worker thread (default 1024; `0` runs on the main thread) |
| `XEZIM_VALUE_TRACE=<substr>[,...]` | Print every committed change of signals whose hierarchical name contains a pattern: time, name, old→new value, dispatch phase, writing process origin (file:line). NBA commits are labeled `nba` |
| `XEZIM_VALUE_TRACE_LIMIT=N` | Cap value-trace output lines (default 20000) |

Example — run the picorv32 testbench against a gate-level netlist:

```bash
./target/release/xezim testbench.v synth.v \
    +firmware=firmware/firmware.hex --max-time 50000000
```

## Command lines from other simulators

One xezim invocation accepts the usual compile and simulate spellings of
commercial simulators, so a flow's existing arguments can be pasted into a
single command. Libraries are not persistent — every run compiles its
sources — so the library options are accepted and ignored:

```bash
xezim -sv +define+UVM_NO_DPI+DEPTH=4 +incdir+tb+rtl -F files.f -work work \
      -c -quiet -lib work hdl_top hvl_top +UVM_TESTNAME=my_test \
      -sv_seed 42 -gDEPTH=8 -l run.log -do "run -all; quit -f"
```

| Spelling | Behaviour in xezim |
|---|---|
| `<top> …`, `work.<top>` | A bare design-unit name that is not an existing file names a top module, same as `-s <top>`. Several give several tops. `<lib>.<top>` works for `work` and libraries named by an earlier `-work`/`-L`/`-lib` |
| `+define+A+B=1`, `+incdir+d1+d2` | Several macros / directories in one flag (as before) |
| `-f <file>`, `-file <file>` | Args file; relative file names resolve as given, else against the args file's directory |
| `-F <file>` | Same as `-f`, except that `+incdir+` directories inside it resolve against the args file's directory first, so `+incdir+.` names the file's own directory |
| `-do "<cmds>"`, `-do <file>` | A subset of the command language: `run -all` (until `$finish` or no events remain), `run <n><unit>` (`fs`…`sec`; several `run`s add up; the run ends at that time, which `final` blocks and the closing line report), `quit`/`exit` (`-f`, `-force`), `do <file>`, separated by `;` or newlines, `#` comments. `log`, `add wave`, `coverage save` and `coverage report` are accepted with one warning each and do nothing. Any other command is an error. A script that quits before any `run` only elaborates. `--max-time` stays a hard cap |
| `-gNAME=VAL` | Sets the default of parameter `NAME` in every module, interface or program that declares it overridable (a `string` parameter takes an unquoted value as text); a value given at an instantiation or by `defparam` still wins. Parameters in generate blocks, and body `parameter`s of a module that has a parameter port list, are local and not reached. `-g/<top>/NAME=VAL` limits it to module `<top>`; deeper paths are ignored with a warning. A name no module declares is warned about and ignored |
| `-GNAME=VAL` | Like `-g`, and it also replaces values given at instantiations and by `defparam` |
| `-sv_seed <n>`, `-sv_seed random` | Same as `+seed=<n>` / `+seed=random` |
| `-sv_lib <name>`, `-sv_root <dir>` | Load `<dir>/<name>.so` as a DPI library (`--dpi-lib`) |
| `-timescale <u>/<p>` | Default timescale for design elements without one (`--module-timescale`) |
| `-l <file>`, `-logfile <file>` | xezim's `-l`: all output goes to the file, none to the terminal |
| `-c` | xezim's args-file flag when a file (or a path) follows; otherwise the batch-mode switch, accepted |
| `-v <file>`, `-y <dir>`, `+libext+` | Library file / directory, as before |
| `+notimingchecks` | As before |
| `-sv12compat`, `-sv17compat` | Same as `--sv2017`. `-sv05compat`/`-sv09compat` do the same with a warning |
| `-work`, `-L`, `-Lf`, `-lib <lib>` | Ignored, with one warning |
| `+cover`, `+cover=<letters>`, `-coverage` | Code coverage (`--code-coverage`): `s`, `b` and `t` select statement, branch and toggle coverage; the bare forms select all three. Other letters (`c`, `e`, `f`, `x`) are ignored with one warning |
| `-sv`, `-mfcu`, `-quiet`, `-64`, `-32`, `-batch`, `-nologo`, `+acc[=…]`, `-<step>args=…` (arguments for a separate optimization step), `-suppress <ids>`, `+fcover`, `-sva`, `-assertdebug` | Accepted, no effect: SystemVerilog is always on, all files share one compilation unit, every object stays visible, assertions and covergroups are always evaluated |
| `-sfcu`, `-t <res>`, `-wlf <file>` | Accepted with a warning: xezim always uses one compilation unit and the finest precision declared in the design, and does not write that waveform file (`--fst` / `--wave` dump waveforms) |

Where a spelling means something else in xezim, xezim's meaning is kept:
`-l` redirects rather than copies the transcript; `-c <file>` reads an args
file. The glued forms `-s<top>`, `-l<file>` and `-f<file>` lose to the
spellings above (`-sv`, `-sv_seed`, `-suppress`, `-lib`, `-logfile`,
`-file`); use `-s <top>` / `-l <file>` / `-f <file>` with a space for such
names.

## Native compilation

Built with `--features jit`, xezim can turn hot bytecode into machine code.

```bash
cargo build --release --features jit

# in-process JIT
XEZIM_JIT=1 ./target/release/xezim <sources> -s <top>

# AOT: generate Rust, build it with rustc, load the result
# (XEZIM_JIT=1 is required — XEZIM_AOT selects the backend, it does not
#  enable native compilation on its own)
XEZIM_JIT=1 XEZIM_AOT=1 ./target/release/xezim <sources> -s <top>

# AOT plus compiled process state machines
XEZIM_JIT=1 XEZIM_AOT=1 XEZIM_PROC_FSM=1 ./target/release/xezim <sources> -s <top>
```

**Whether it pays depends on the design — measure before adopting it.** Same
binary, warm native cache, wall-clock:

| | interpreter | `XEZIM_JIT` | `+AOT` | `+AOT +PROC_FSM` |
|---|---|---|---|---|
| Ibex CoreMark | 50.8s | **39.0s** (−23%) | **38.8s** | 39.1s |
| C906 memcpy ×100 | 49.3s | 55.7s (**+13%**) | 49.5s | 47.8s (−3%) |

The C906 loss is entirely compile time, not slower simulation: JIT takes its
simulation phase from 43.6s to 42.9s but spends 7.0s more compiling, because
the design has 35,267 combinational entries to Ibex's 1,553 and the per-block
cost is amortized ~37× less. Compiling only the hot subset does not rescue it
— the eval distribution is steep enough (15% of entries carry 99.2% of
evaluations) that a threshold looked promising, but JIT is only worth 2.3% of
C906's simulation phase in the first place, and on Ibex the warmup needed to
measure hotness costs more than the compile it saves. Rule of thumb: native
compilation pays on designs with relatively few, very hot blocks.

The AOT backend covers combinational entries, edge-sensitive blocks, and — when
`XEZIM_PROC_FSM=1` is also set — process FSMs. Blocks it cannot lower (values
wider than 64 bits, unsupported opcodes, X/Z-carrying shapes) stay on the
interpreter, so coverage is partial by design; `XEZIM_JIT_VERBOSE=1` prints the
`[AOT] … compiled N/M` summary.

Generating and compiling that Rust is the dominant cost on a first run — minutes
on a large SoC — so the resulting library is cached under `$XEZIM_CACHE_DIR`,
`$XDG_CACHE_HOME/xezim/native`, or `~/.cache/xezim/native`, keyed on the
generated source, `XEZIM_AOT_OPT`, and the xezim build. Repeat runs load the
cached `.so` directly. Set `XEZIM_NO_NATIVE_CACHE=1` to force a rebuild, and
`XEZIM_AOT_OPT=0` to trade steady-state speed for a faster build.

## Warm design cache

Simulation mode stores a content-addressed elaborated design and compiled
combinational worklist after the first run, then reuses both on identical later
runs. A cache hit skips parsing, elaboration, and combinational dependency-index
construction, while simulator state, plusargs, time-zero initialization, and
event scheduling are rebuilt for every invocation. Timing-annotated and UDP
designs conservatively rebuild the worklist. The key covers source and library
contents, defines, include paths, top selection, language/strictness, timescale
and delay settings, and the xezim executable build.

The default directory is `$XEZIM_CACHE_DIR`, then
`$XDG_CACHE_HOME/xezim/designs`, then `$HOME/.cache/xezim/designs`. Use
`--cache-dir` for a workload-local cache or `--no-cache` for a cold run. With
`--verbose`, xezim prints `[CACHE] miss`, `[CACHE] stored`, or `[CACHE] hit` on
stderr.

## Reducing a multi-file build

Three flags answer the questions that come up when a large `-f` build does not
behave: *which* files were read, *what* each contributed, and *what does the
code look like after preprocessing*.

```bash
xezim -f build.args --dump-files-list          # the resolved file list, then exit
xezim -f build.args -s testbench --verbose     # each file as it is parsed, and what it defined
xezim --parse -f build.args -s testbench --dump-merged-sv repro.sv
```

`--dump-merged-sv` writes every source into one self-contained `.sv` with
`` `ifdef `` branches resolved, macros expanded and `` `include ``s inlined — a
125-file build becomes a single re-runnable file. Given `-s <top>` it keeps only
the files that top actually needs, which is what makes the result small enough
to hand to someone else.

Two properties are worth knowing before relying on it:

* **The reduction is per file, not per module.** A file defining both a module
  you need and one you do not drags the second one's dependencies in too.
* **The closure is lexical and runs before parsing**, so the dump still works on
  a design that does not elaborate — the case the flag exists for. It is
  conservative in the safe direction: it may keep a file more than strictly
  needed, never one fewer. Files that declare no design unit at all (a
  file-scope `typedef`/function, a top-level `bind`) are always kept, since
  nothing references them by name and dropping them would change behaviour.

Note `--parse` above: the dump is produced before elaboration, so a design whose
elaboration takes minutes still dumps in seconds. Only the step that appends
adopted `-v`/`-y` library files needs `--compile` or `--simulate`.

## Module-timescale extension

`--module-timescale` is an xezim-specific command-line extension. It assigns a
time unit and precision to module *definitions* that have **no explicit
source-level timescale**, without changing the semantics of the source. It is
handy for retrofitting a timescale onto legacy RTL that omits one, or onto a
mix of files where only some carry `` `timescale ``.

```bash
# Every module without an explicit timescale gets 1ns/1ps:
xezim --module-timescale 1ns/1ps design.sv

# Only the listed definitions (comma-separated), 10ns/1ns:
xezim --module-timescale cpu,cache=10ns/1ns design.sv

# Repeatable; the named form wins over the global one:
xezim --module-timescale 1ns/1ps --module-timescale mem_ctrl=1ps/1fs design.sv
```

**A module has an explicit source-level timescale** — which the option never
overrides — when it has a `timeunit`/`timeprecision` declaration, **or** a
`` `timescale `` directive is active where it is declared (`` `resetall ``
clears that). Effective precedence, highest first:

1. module-local `timeunit` / `timeprecision`
2. an active `` `timescale `` directive
3. a named `--module-timescale mods=<unit>/<prec>`
4. a global `--module-timescale <unit>/<prec>`
5. the 1ns / 1ns default

The precision must be equal to or finer than the unit (`1ns/1ps` is legal,
`1ps/1ns` is an error). Two *different* named assignments for the same module
are an error; an unmatched name, or one that lands on a module that already has
an explicit timescale, is a warning (the assignment is ignored). Assignments
apply to a definition, so every instance of it shares the timescale.

Sub-nanosecond precision is honoured — the simulation tick is the finest
precision declared anywhere in the design, down to `fs`. `--max-time` is
independent of that: it is given in nanoseconds and converted to the tick, so
`--max-time 100` stops at 100 ns whether the design runs at `1ns` or `1fs`
precision. What a finer precision does change is the *number of ticks* covered,
and hence the wall-clock cost of reaching the same simulated time. Reported
times (`$time`, the closing `Simulation finished at time …`) are in ticks, so
the same run prints `100` at `1ns/1ns` and `100000` at `1ns/1ps`.

Because the cap is held in whole nanoseconds, a sub-nanosecond `--max-time`
(`--max-time 1ps`) is rejected rather than silently rounded to zero. To stop a
run as early as possible, prefer `--parse` or `--compile`, which never start a
simulation at all.

### Inspecting resolved timescales

`--dump-timescales` prints the resolved timescale of every module *before* the
run — no source `$printtimescale` calls required. It reports each definition's
`` `timescale `` semantics (an explicit/`--module-timescale` value, or the
`1ns/1ns` default when a module has none) and flags the modules that carry no
`` `timescale ``. Combine it with `--module-timescale` to confirm an assignment
landed where you intended.

```bash
$ xezim --dump-timescales design.sv
=== module timescales (3 modules) ===
  cache                        10ns / 1ns
  cpu                          1ns / 1ps
  glue                         1ns / 1ns   (no `timescale — 1ns/1ns default)
======================================
```

A flagged module also emits the `has no timescale directive` warning in a
mixed-timescale design; give it a source `` `timescale `` or a
`--module-timescale` assignment to resolve it. (The default is tool-defined by
IEEE 1800 §3.14.2.2; xezim uses `1ns/1ns` for both delays and `$realtime`, so
an untimed module's `#1` is one nanosecond — declare a timescale explicitly when
you mean something else.)

---

# Long-Term Vision

This project explores several long-term ideas:

* **AI-assisted EDA development**
* **Rapid simulator prototyping**
* **Cloud-scale simulation**
* **Distributed multi-CPU simulation**

The goal is to investigate whether modern software and AI tools can dramatically accelerate the creation of chip design infrastructure.

---

# License

Apache License 2.0

See the `LICENSE` file for details.

---

# Contributors

xezim is developed in the open, and a number of people have improved it through
pull requests. Thank you to everyone who has contributed — bug fixes, features,
tests, and tooling all move the project forward:

* **Thomas Burg** — class-system and UVM fixes: static-property chains through
  object handles (§8.25), associative-array method dispatch and ref-writeback,
  `ClassName::static_prop` access, parser-gap self-tests, test-harness
  hardening, per-process bookkeeping for methods that park mid-body, the
  condition-waiter drain de-duplication, and the NBA-region lane in the
  `--max-time` hang report.
* **Vrajesh Prakhya** — real-number modelling coverage: Verilog-AMS `wreal`
  nets resolved by summing, user-defined nettypes across the hierarchy and in
  packages (§6.6.7, §6.6.8), real-ness of members projected from call results,
  negative-test registrations, and the diagnosis that `cover property` sites
  were tallied as failing assertions.
* **Oscar Gustafsson** — expanded VPI functionality (`vpi_get_value`,
  `ObjectValType`), CI setup, and clippy cleanups.
* **Chen Ben Haroosh** — submodule-inline generate-for elaboration: genvar-
  dependent declarations and `parameter type` default resolution, plus the
  accompanying SystemVerilog compliance cases.
* **Jayaraman RP** — cross-platform installation scripts, including the macOS
  installer with UVM setup.
* **Ganesh T S** — wide-number parsing for `$sscanf`/`$fscanf`/`$value$plusargs`
  and the string conversion methods (arbitrary-precision decimal in the core),
  loop-variable scoping against same-named identifiers in child instances, and
  detailed self-checking bug reports for struct and class member access.

New contributors are welcome — see [Contributing](#contributing).

---

# Acknowledgements

* Icarus Verilog project for the public test suite
* The Rust community
* Open-source EDA projects
