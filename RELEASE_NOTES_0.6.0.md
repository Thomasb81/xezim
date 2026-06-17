# xezim 0.6.0 — Release Notes

xezim is a from-scratch SystemVerilog elaborator + event-driven simulator
written in Rust. 0.6.0 is a large step in language coverage, full-chip
scalability, and competitiveness against established open-source tools.

## Highlights

- **Competitive sv-tests compliance.** On the full sv-tests corpus (4772 tests),
  xezim now passes **4364 (91.4%)** — within ~1 point of Verilator 5.044
  (92.4%) and well ahead of Icarus Verilog 12.0 (80.0%), measured through
  sv-tests' own `make report` (should_fail-aware verdicts).

  | simulator | pass | rate |
  |---|---|---|
  | Verilator 5.044 | 4409 | 92.4% |
  | **xezim 0.6.0** | **4364** | **91.4%** |
  | Icarus Verilog 12.0 | 3818 | 80.0% |

- **Full-chip RTL elaborates.** All **375/375** sv-tests generated RTL cores
  elaborate: BaseJump STL (360), black-parrot (6, all configs), ariane/CVA6 (4),
  scr1, veer-el2, fx68k, rsd.

- **Runs a real CPU end-to-end.** The XuanTie C910 (dual-core, L2, AXI SoC)
  simulates the memcpy workload to completion and self-checks `TEST PASSED`.

## Language & feature coverage (new/expanded in 0.6)

- **Verilog structural primitives.** Gate & switch primitives —
  `nmos/pmos/cmos` (+`r` variants), `tran/tranif0/1` (+`r`), `pullup/pulldown`,
  and the previously-dropped `bufif/notif` — elaborate to functional models.
- **Drive/charge strengths** on gates, continuous assigns, and net declarations
  (`wire (strong1, weak0) w = …;`) — parsed and accepted.
- **User-defined primitives** (`primitive … endprimitive`) — accepted so their
  instantiations resolve.
- **specify blocks** — accepted (path delays not modeled).
- **`var` declarations** in every context (implicit-type `var x;`,
  `input var int x`, non-ANSI `input var …`).
- **Enum element ranges** (§6.19.1): `UVAL[256]`, `JMP[6:8]`, `P[5]=PVALUE`
  expand to the indexed member set.
- **Non-ANSI split ports** (§23.2.2.1): a separate data-type and direction
  declaration for the same port (`byte x; output x;`) now merge correctly.
- **`$unit`-scope parameters** are properly shadowed by module-local
  parameters (§3.12.1).
- **Per-module `timescale`** (§22.7) with delays pre-scaled to the design's
  finest precision; `$time` reported in ns.

## Correctness / error detection

- §23.2.2.1 conflicting non-ANSI port type redeclarations and §23.2.2.3
  `inout var` are now diagnosed as errors.
- §23.3.2 library directories (`-I`) now supply only *instantiable* units
  (modules/interfaces/programs) — packages/classes/typedefs are no longer
  blanket-imported from incdirs, fixing global-scope contamination when an
  incdir holds many unrelated files.

## Performance & memory

- **Full-chip elaboration memory cut ~62%.** C910 memcpy peak RSS dropped from
  9.2 GB to **3.5 GB** by sharing each module instance's rewrite context across
  its pending behavioral items (`Rc<RewriteCtx>`) instead of cloning it per
  item, and by freeing the elaboration-time signal map once the simulator's
  parallel signal vectors are built. Wall time improved ~14% as a side effect.
- **~2.8× faster than Icarus** on C910 memcpy end-to-end (217 s → compile+sim;
  iverilog+vvp 608 s), trading higher memory for speed.
- **Combinational levelization** topologically orders combinational logic so
  feed-forward chains settle in a single delta cycle; the cycle-break selector
  is now O(V+E) (was O(n²) on designs with many small combinational loops).
- 4-state `Value` is 32 bytes with inline storage for ≤64-bit signals (no
  per-element heap), keeping large memory arrays compact.

## Tooling

- Integrated as an sv-tests runner (`tools/runners/Xezim.py`); run with
  `make RUNNERS_FILTER=Xezim report` or the bundled `run_xezim_suite.sh`.
- Modes: `--preprocess`, `--parse`, `--compile` (elaborate), `--simulate`.
- `-V` reports the version; lossy-UTF8 source reading; `-f`/`-c` filelists.

## Compatibility notes

- `xezim-core` is now a sibling **path dependency** rather than a vendored git
  submodule. Building `xezim` standalone expects `../xezim-core` to be present.

## Known limitations

- ~264 sv-tests `should_fail` cases that Verilator rejects are still accepted
  (error-detection gaps, deferred to avoid false-positives on valid code).
- specify path delays, SDF timing, and UDP truth-table semantics are accepted
  but not functionally modeled.
- Large unpacked memory arrays are allocated to full declared depth (sparse
  backing is a planned memory optimization).
