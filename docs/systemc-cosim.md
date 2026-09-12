# SystemC co-simulation

xezim can simulate a SystemVerilog design and a SystemC model **in one
process**, with either side as the simulation top. The co-simulation master
is **inferred from the C++ source** — a TU defining `xsc_model_build` is a
passive model (SystemVerilog top), a TU defining `sc_main` owns the run
(SystemC top) — the same spirit as auto-elaborating an uninstantiated root
module. Signals whose registry name matches an SV signal name are attached
automatically, so the common cases need **no `--sc-*` flags beyond the
source itself**:

| Mode        | Top                    | Time master | Typical use                          |
|-------------|------------------------|-------------|--------------------------------------|
| SV top      | SystemVerilog testbench / DUT | SystemVerilog kernel, SystemC driven by `$sc_step` / exchanges | SystemC IP (TLM, crypto, DSP) inside an SV environment |
| SC top      | `sc_main` model        | SystemC kernel; the SV kernel is driven per slice | Reuse an existing `sc_main` executive; SV acts as the DUT |

SystemC support is **runtime-gated**: it is always compiled in but only
activates when `--sc-source` / `--sc-lib` is passed. Runs without those
flags are byte-for-byte identical to plain xezim.

## Requirements

- A SystemC installation (2.3+ or 3.x) with `SYSTEMC_HOME` pointing at it
  (`$SYSTEMC_HOME/include/systemc` must exist; the library is found under
  `$SYSTEMC_HOME/lib*`).
- `g++` (the model translation unit is compiled at run time and `dlopen`ed).

## The shared API: `sc_api.h`

The shim header `src/sc_shim/sc_api.h` ships with xezim. Include it from the
C++ translation unit and bind every signal that crosses the boundary:

```cpp
#include <systemc>
#include "sc_api.h"   // -I <xezim>/src/sc_shim

SC_MODULE(Incr) {
  sc_core::sc_in<bool>       en{"en"};
  sc_core::sc_in<uint64_t>   din{"din"};
  sc_core::sc_out<uint64_t>  cnt{"cnt"};
  void run() {
    sc_core::sc_signal<bool>&  e = *en;   // illustrative
    (void)e;
  }
  SC_CTOR(Incr) { SC_METHOD(run); }
};

int sc_main(int, char**) { /* build, bind, run */ }
```

- `xezim::xsc_bind(name, signal)` registers a signal in the co-simulation
  **registry** under `name`. SV code references the same name.
- Widths ≤ 64 bits only (enforced with an error). `sc_lv` bits map to 2-state:
  `X`/`Z` read as `0`; SV `X`/`Z` written into an `sc_lv` become `0`.

### SV top: build a SystemC model

Provide an `extern "C" void xsc_model_build()` that constructs the model and
binds registry signals — instead of `sc_main`:

```cpp
extern "C" void xsc_model_build() {
  static Incr incr("incr");
  xezim::xsc_bind("en",  incr.en);
  xezim::xsc_bind("din", incr.din);
  xezim::xsc_bind("cnt", incr.cnt);
}
```

```console
$ xezim --simulate -s tb --sc-source model.cc tb.sv
```

The master is inferred as **sv** because the TU defines `xsc_model_build`.

If a registry name matches an SV signal name exactly (e.g. registry `en` and
SV signal `en`), the pair is attached **automatically** and synced both ways
at every exchange — no `$sc_*` call needed. The `$sc_*` tasks below remain
for renamed signals and for code that wants explicit direction:

```systemverilog
$sc_export_sig("en",  en);    // SV -> SystemC (stage a value)
$sc_import_sig("cnt", cnt);   // SystemC -> SV (pull the current value)
$sc_step(10);                 // advance the SystemC kernel exactly 10 ticks
$sc_finish;                   // end the SystemC kernel early
```

`$sc_step(n)` advances SystemC by exactly `n` ticks; after it returns,
imports reflect the SystemC state at `n` ticks later than before. The call
also stages exports and applies the resulting imports.

### SC top: `sc_main` owns the run

Write a normal `sc_main`, but call `xsc_cosim_run()` **instead of
`sc_start()`** — xezim drives the kernel one slice at a time and the
`sc_start()` calls inside the slice loop execute on `sc_main`'s own thread
(the only context the standard allows):

```cpp
int sc_main(int, char**) {
  sc_core::sc_signal<uint64_t> cmd{"cmd"}, resp{"resp"};
  Master m("master");
  m.cmd(cmd);  m.resp(resp);
  xsc_bind("cmd",  cmd);
  xsc_bind("resp", resp);
  xsc_cosim_run();          // NOT sc_start()
  return 0;
}
```

```console
$ xezim --simulate -s dut --sc-source sc_master.cc dut.sv
```

The master is inferred as **sc** because the TU defines `sc_main`. The
registry signals `cmd`/`resp` match the DUT's port names `cmd`/`resp`
exactly, so they are auto-attached — bidirectionally: xezim stages the SV
value into SystemC before each slice and applies the SystemC value back
after it. A signal driven from only one side behaves exactly like a
directed mapping (writing the current value is a no-op); if both sides
drive, the SystemC value from the just-completed slice wins.

To attach a signal whose names differ on the two sides (here `cmd_in` →
registry `cmd`), map it explicitly:

```console
$ xezim --simulate -s dut --sc-source sc_master.cc \
    --sc-map 'cmd_in=cmd:in' --sc-map 'resp_o=resp:out' dut.sv
```

`--sc-map '<sv_name>=<sc_name>:<dir>'` maps a top-level SV signal to a
registry signal; `dir` is `in` (SystemC drives the SV signal) or `out` (the
SV signal drives SystemC). Repeat the flag per signal. Explicit rows take
precedence over name-matched rows for the same registry name.

A prebuilt model can be passed instead of sources with
`--sc-lib <path>.so` (built from the user TU together with
`src/sc_shim/sc_shim.cc`).

## Time model

- The SystemC time resolution is set to xezim's tick length at model
  initialization; one `sc_time` value unit equals one xezim tick.
- **SV top**: SystemC time advances only inside `$sc_step(n)`.
- **SC top** (one-slice lookahead): xezim stages its export values, the
  SystemC side applies them and runs to its next pending activity (bounded
  by `XEZIM_SC_QUANTUM`, default 1000 ticks), then xezim adopts the SystemC
  time as an upper bound (`cap`) and applies the imported values. Consequences:
  - An SV signal driven by SystemC changes at xezim's current time when the
    slice closes, never later than the SystemC time that produced it.
  - A value SystemC reads mid-slice reflects the SV state as of the *start*
    of that slice (replies lag one slice).
  - Both kernels quiescent ⇒ the run ends. The t=0 SystemVerilog slot runs
    before the first exchange so `@`-arms don't swallow initial imports.

## Traces

VCD, XTrace and FST dumps include the SystemC registry under a
`systemc_bridge` scope, merged with the SystemVerilog signal data. SystemC
mirror values update at each exchange (SV top: `$sc_step`/exchange; SC top:
every slice), so a trace shows both kernels on one timeline.

## Exit conditions

- SV top: `$sc_finish`, or the SV run ends (kernel teardown stops SystemC).
- SC top: `sc_main` returns, the SV run ends (`$finish` / `--max-time`), or
  both kernels are quiescent.
- If the model fails to compile or load, xezim reports the compiler/loader
  error and the SystemVerilog side runs standalone (a `--sc-top sc` run whose
  model cannot host the master link exits with an error instead).

## Flag reference

| Flag / variable | Meaning |
|---|---|
| `--sc-source <file.cc>` | C++ source compiled with the shim into the model `.so` (repeatable). Its entry point (`xsc_model_build` vs `sc_main`) infers the master. |
| `--sc-top <sv\|sc>` | Override the inferred master (rarely needed). |
| `--sc-include <dir>` | Extra `-I` path (repeatable). |
| `--sc-cxxflag <flag>` | Extra compiler flag (repeatable). |
| `--sc-lib <path>` | With `--sc-source`: an extra link library (`-lfoo` / `-lfoo.so` / a directory → `-L`). Without sources: a prebuilt model `.so` to load directly (repeatable). |
| `--sc-map '<sv>=<sc>:<in\|out>'` | Explicit signal mapping for renamed signals (repeatable); everything else name-matches automatically. |
| `XEZIM_SC_QUANTUM` | SC-master slice bound in ticks (default 1000). |

`SYSTEMC_HOME` must be exported for any `--sc-*` run.
