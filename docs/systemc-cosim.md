# SystemC co-simulation


## Auto-derived wrapper (no bind API) — SystemC top

The recommended way to integrate a pure SystemVerilog DUT into a SystemC
model is the **auto-derived wrapper**: xezim reads the SV DUT's top-module
ports after elaboration and generates a C++ `SC_MODULE` (`XezTop`) whose
public `sc_signal` members mirror those ports. The SystemC testbench needs
to know nothing about co-simulation:

```systemverilog
// dut.sv — ordinary SystemVerilog, no SystemC knowledge
module sv_top(input logic [7:0] din, output logic [7:0] dout);
  assign dout = din + 8'd1;
endmodule
```

```cpp
// tb.cc — ordinary SystemC
#include <systemc>
#include <cstdio>
#include "xez_top.h"     // generated automatically; -I added for you

int sc_main(int, char**) {
  XezTop dut("dut");
  dut.din.write(41);
  sc_start();            // interposed: drives the SV kernel per slice
  std::printf("%llu\n", (unsigned long long)dut.dout.read());
  return 0;
}
```

```console
$ xezim --simulate -s sv_top --sc-source tb.cc -I <xezim>/src/sc_shim dut.sv
```

- `-s sv_top` names the SV DUT; xezim generates `XezTop` from its ports
  (name, direction, width). Directions map to `sc_signal` backing members: SV
  input → read from SystemC, SV output → written by SystemC.
- The C++ source defines `sc_main`, so the master is inferred as **sc** (SC
  instantiates SV).
- `#include "xez_top.h"` resolves because xezim adds the generated-header
  directory to the compile `-I` path and compiles the generated `XezTop`
  implementation with your testbench into the model `.so`.
- `sc_start(...)` is `#define`d onto the co-simulation driver, so your
  `sc_main` stays a plain SystemC program; arguments are accepted and
  dropped.

Supported port types: 2-state, ≤ 64 bits (`logic`, `bit`, `logic [63:0]`);
wider ports are not exposed through the wrapper. This mirrors the reference
simulators' pure-SystemC / Verilator `Vtop` integration.

See `examples/sc_cosim/` for runnable reference examples.

---

## Manual API (`xsc_bind` / `$sc_*`)

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

### Netlist leaf (Mode B): SV instantiates SC inside its body

Beyond the boundary flow above, a pure-SystemC module can be instantiated
**directly in the SystemVerilog netlist** — the reference `vlog_sc` shape
(`inc_leaf u(.clock(clk), .din(din), .dout(dout))` with **no** SV
`module inc_leaf` anywhere):

```console
$ xezim --simulate -s sv_leaf_top -I <xezim>/src/sc_shim \
    --sc-module inc_leaf --sc-source inc_leaf.cc sv_leaf_top.sv
```

How it works:
1. **Reflect** — xezim scans `inc_leaf.cc` and discovers the SystemC module's
   ports (`sc_in<bool> clock`, `sc_in<sc_uint<8>> din`, `sc_out<sc_uint<8>>
   dout`) and their template types from the source text.
2. **Synthesize a placeholder** — an empty-body SV module `inc_leaf` with the
   matching ANSI port header is injected before elaboration, so the SV
   netlist elaborates cleanly with no "module not found".
3. **Auto-bind at the instantiation site** — the bridge discovers the SC
   instance in the netlist (`u`), resolves every port to an SV net (inputs via
   the elaborator's `input_port_actuals` / connection assigns, outputs via
   `port_aliases`), and binds the SC module's own backing port signals into
   the registry. The elaborator's existing output-port continuous assigns
   propagate SC-driven values to the actuals.
4. **Auto-step (reference-parity timing)** — each time SV time advances, the
   bridge runs an initial exchange at time 0 (exposing the SC leaf's
   initialized outputs on the netlist at t=0 and establishing its input
   baseline), then after each new SV slot settles it advances the SystemC
   kernel by exactly the elapsed ticks — **delivering the post-edge clock /
   inputs** — and pulls the leaf's outputs back into the *same* slot. So a
   posedge-{triggered} SC output (e.g. `dout = din + reg` on the clock edge)
   appears in the SV slot of that edge, byte-for-byte like the reference
   simulators, with an initial `dout=0` at t=0. The SV testbench needs
   **no `$sc_step`**.

`sc_uint<8>` is *not* a C++ integral, so the backing signal is bound with an
overloaded `xsc_bind(sc_signal<sc_dt::sc_uint<W>>&)` (and `sc_bv<W>`/`sc_lv<W>`)
rather than the generic template. The backing signals are emitted with the
SystemC datatype **namespace-qualified** (`sc_dt::sc_uint<8>` / `sc_dt::sc_bv<8>`
/ `sc_dt::sc_lv<8>`): the generated model translation unit only

```cpp
#include <systemc>
```

and does not inherit a `using namespace sc_dt`, so an unqualified `sc_bv<8>`
would not compile. See `examples/sc_cosim/sv_leaf_top.sv` + `inc_leaf.cc`
(verified end-to-end: `SV_LEAF dout=43`), and the `sv_leaf_sc_bv_lv_ports`
self-test for an `sc_bv`/`sc_lv` crossing.

### Reference-simulator validation

The co-simulation is validated byte-for-byte against a reference simulator's
own SystemC examples (`/examples/systemc/`). xezim reproduces:

- **`vlog_sc`** (SV testbench instantiates a hierarchical SystemC `ringbuf`):
  `** NOTICE ** at 99800 ns: Data value not expected.` and
  `** RESTORED ** at 202800 ns: Data returned to expected value.` at the same
  times, and the same `dout` values at the same edges (`t=0 dout=0`,
  posedge `dout=43`, held thereafter). Two things were required to reach
  this parity: (a) the port-reflection scanner must **exclude internal
  `sc_signal` members** from the reflected port list (a hierarchical SC
  module wires its children with internal signals that are *not* bindable
  ports), and (b) the reference testbench connects `.reset` to the SC port
  `Reset` — the reference simulator matches foreign ports case-insensitively, while xezim
  is currently case-sensitive (a known limitation; the scratch run adapts
  the connection to `.Reset`).
- **`param_passing/vlog_to_sc`** (SV instantiates SC with `counter_size`,
  `buffer_size`, `real_param`, `exp1_param`, `exp2_param`, `str_param`, and
  an X/Z `range_param`): all seven parameter values match the reference
  `results` file byte-for-byte (`counter_size=4, buffer_size=16,
  real_param=2.6, exp1_param=6, exp2_param=10, str_param=Hellow World,
  range_param=001100xz`) — including the X/Z range preserved as its binary
  string — plus the same `NOTICE`@99800/`RESTORED`@202800 behavior. Reaching
  parity required aligning `sc_api.h`'s `sc_get_real_param(name, int*)` and
  `sc_get_string_param(name, char, int*)` signatures with the IEEE-1666 API
  the reference simulator ships (see § Parameter passing).

A Docker image (`refscmb`, ubuntu:18.04) hosts a reference simulator's bundled
gcc-7.4.0 against the reference's glibc-2.27 so the reference runs compile on
this host. The reference trace is captured under `vsim_full.log`.

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

To attach a signal whose names differ on the two sides (here `cmd_in` on the
SystemVerilog side vs `cmd` in the SystemC module), use the low-level
explicit hooks `$sc_import_sig` / `$sc_export_sig` from SystemVerilog to bind
the SC-side registry name explicitly (the auto-derived wrapper, by contrast,
attaches purely by matching name).

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

## Parameter passing

Matching the reference simulators' `param_passing` examples, parameters can
cross the boundary in either direction:

**SystemVerilog → SystemC (implemented).** In SV-master mode the SV top
module's `parameter`s are forwarded into the co-sim parameter registry
*before* the SystemC model's constructor runs, so a pure-SystemC module reads
them with the standard IEEE-1666-style functions — source-compatible with
the reference `vlog_to_sc` example (signatures match a reference simulator's bundled
SystemC `sc_get_*_param` accessors):

```cpp
int         n = sc_get_int_param("counter_size", &ok);   // &ok is int*, 1 if present
int         ok2 = sc_get_param("buffer_size", buffer_size);   // returns 1 if present
std::string s = sc_get_string_param("str_param", 'a', &ok);   // char fmt + int* status
int         ok3 = sc_get_param("str_param", s, 'a');
```

The xezim-native fallback forms remain available (`xsc_get_int_param(name,
fallback, &ok)`, `xsc_get_real_param`, `xsc_get_string_param`), and all of
them read elaborated values — after any `-G<NAME>=<value>` CLI override (the
SV-side `#(.name(value))` analog). See `examples/sc_cosim/sv_param_top.sv` +
`sc_param_sink.cc`.

`int`, `real` and `string` parameter types are supported. A vector parameter
that contains unknown/high-impedance bits (e.g. `parameter [7:0] range_param
= 8'b001100xz`) is forwarded as its width-padded binary string preserving the
`X`/`Z` (matching the reference, which reports `range_param=001100xz`), rather
than being collapsed to a numeric `0`. Values are only treated as strings when
every byte is printable ASCII.

The registry backing store lives in `sc_shim.cc` (`xsc_set_param_count` /
`xsc_set_param_at` C ABI + the `xsc_get_*_param` readers declared in
`sc_api.h`).

**SystemC → SystemVerilog (infrastructure in place).** The SV-side override
plumbing (`ParamOverride` / `set_param_overrides` / `-G`) and the SC-side
registry both exist, but SC-top runs set parameters inside the C++ main
entrypoint, which executes *after* the SV worker has already elaborated the
top. Making SC-set parameters affect the SV top needs a pre-elaboration
rendezvous on the shared link — follow-up work. Use-case 4 (SV reads into SC)
is the currently-exercised direction.

## Exit conditions

- SV top: `$sc_finish`, or the SV run ends (kernel teardown stops SystemC).
- SC top: `sc_main` returns, the SV run ends (`$finish` / `--max-time`), or
  both kernels are quiescent.
- If the model fails to compile or load, xezim reports the compiler/loader
  error and the SystemVerilog side runs standalone (an SC-master run whose
  model cannot host the master link exits with an error instead).

## Flag reference

| Flag / variable | Meaning |
|---|---|
| `--sc-source <file.cc>` | C++ source compiled with the shim into the model `.so` (repeatable). Its entry point defines the master. |
| `--sc-include <dir>` | Extra `-I` path (repeatable). |
| `--sc-cxxflag <flag>` | Extra compiler flag (repeatable). |
| `--sc-lib <path>` | With `--sc-source`: an extra link library (`-lfoo` / `-lfoo.so` / a directory → `-L`). Without sources: a prebuilt model `.so` to load directly (repeatable). The prebuilt master is inferred by probing its dynamic symbols. |
| `--sc-module <Name>` | SV-master auto-derive: instantiate the user SystemC module in the generated model harness. When the SV netlist itself instantiates `<Name>` (Mode B netlist leaf), its ports are reflected from `--sc-source` and a placeholder is synthesized, with every port auto-bound and the SC kernel auto-stepped per SV time advance. |
| `XEZIM_SC_QUANTUM` | SC-master slice bound in ticks (default 1000). |

The co-simulation **master** (which kernel owns the clock) is never user-picked:
`--sc-top` was removed in favour of auto-inference. A prebuilt `.so` (via
`--sc-lib`) is probed with `nm -D`: a **strong** `sc_main` symbol ⇒ SystemC is
master; a defined `xsc_model_build` ⇒ SystemVerilog is master (the shim's weak
`sc_main` fallback is correctly ignored). For `--sc-source` text, a **definition**
of `sc_main` (a `{` body, not a header prototype, and not inside a comment or
string) ⇒ SC master; a definition of `xsc_model_build` ⇒ SV master. The signal
crossing is bound by name automatically, so no explicit `--sc-map` is needed.
