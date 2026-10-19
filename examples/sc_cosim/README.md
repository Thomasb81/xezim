# SystemC ↔ SystemVerilog seamless co-simulation examples

These examples demonstrate xezim's **auto-derived interop wrapper**: pure
SystemVerilog + pure SystemC, no bespoke bind API.

`sv_incr` / `sc_tb_sv_incr.cc` — combinational, **works end-to-end** (SC top):

```console
SYSTEMC_HOME=<prefix> xezim --simulate -s sv_incr \
  -I <xezim>/src/sc_shim \
  --sc-source sc_tb_sv_incr.cc sv_incr.sv
```
```
TAG_PASS dout=42
```

`sv_adder` / `sc_tb_sv_adder.cc` — registered RTL DUT driven by an SC
testbench. Uses the same seamless flow; the registered `always_ff` output is
stuck at its reset value because the SC-top slice driver does not yet commit
edge-triggered NBA writes on imported clock edges (same pre-existing
limitation as the manual `xsc_cosim_run` flow — see issue notes). The wrapper
itself crosses values correctly; combinational paths are fully verified.

`sv_param_top.sv` / `sc_param_sink.cc` — **parameter passing (SV master →
SystemC module), works end-to-end**: a pure SV top's `parameter`s (int, real
and string) are forwarded into the co-sim parameter registry *before* the pure
SystemC module's ctor runs, so the model reads them with the standard
`sc_get_int_param` / `sc_get_real_param` / `sc_get_string_param` — the same
`vlog_to_sc` parameter-passing semantics as the reference simulators:

```console
SYSTEMC_HOME=<prefix> xezim --simulate -s sv_param_top \
  -I <xezim>/src/sc_shim \
  --sc-module sc_param_sink --sc-source sc_param_sink.cc \
  sv_param_top.sv
```
```
SV_PARAM counter_size=4 width=8 scale=2.5 tag=sq
SC_PARAM counter_size=4 width=8 scale=2.5 tag=sq
```

Override any parameter with `-G<NAME>=<value>` (the SV-side `#(.name(val))`
analog); both sides stay in agreement:

```console
... --sc-module sc_param_sink ... -Gcounter_size=9 -Gscale=3.5 -Gtag="SHA256" \
    sv_param_top.sv
```
```
SV_PARAM counter_size=9 width=8 scale=3.5 tag=SHA256
SC_PARAM counter_size=9 width=8 scale=3.5 tag=SHA256
```

`sv_leaf_top.sv` / `inc_leaf.cc` — **netlist leaf (Mode B), works end-to-end**
(SV top directly instantiates a pure SystemC module in its netlist — the
`vlog_sc` use-case shape). There is **no** `module inc_leaf` in the SV source;
xezim reflects the SystemC module's ports from `inc_leaf.cc`, synthesizes a
placeholder for elaboration, and auto-steps the SystemC kernel at each SV time
advance, so the SV testbench needs **no `$sc_step`**:

```console
SYSTEMC_HOME=<prefix> xezim --simulate -s sv_leaf_top \
  -I <xezim>/src/sc_shim \
  --sc-module inc_leaf --sc-source inc_leaf.cc sv_leaf_top.sv
```
```
SV_LEAF dout=43 (expect 43)   // SC registers din+1 = 42+1 on posedge clk
```

## Use-case matrix

| Use case | Example | Status |
|----------|---------|--------|
| 1. Pure SystemVerilog | `sv_incr.sv` / `sv_adder.sv` (the DUT files) | works standalone |
| 2. Pure SystemC | `sc_param_sink.cc` (a standalone SC_MODULE) | works standalone |
| 3. SystemC → SystemVerilog (SC top instantiates SV) | `sv_incr`/`sc_tb_sv_incr.cc`, `sv_adder` | works (combinational verified) |
| 4. SystemVerilog → SystemC (SV top instantiates SC) | `sv_param_top.sv`/`sc_param_sink.cc`; `sv_leaf_top.sv`/`inc_leaf.cc` (netlist leaf, Mode B) | works (port + param passing + netlist-leaf verified) |
| 5. Sandwich (SC→SV→SC / SV→SC→SV) | — | blocked: needs SV-netlist SC-instance discovery (see README) |

The master is inferred from the C++ source: a TU defining `sc_main` is SC
master; a TU defining a plain `SC_MODULE` (no `sc_main`/`xsc_model_build`) is
the passive SC side under an SV master via `--sc-module <Name>`.

The passive SC side can be instantiated in **either** of two ways:

* **Boundary** (`sv_param_top.sv`): the SV top's own ports match the SystemC
  module's ports by name/width; the SC module is instantiated at the boundary
  and the SV top's `parameter`s are forwarded into the registry.
* **Netlist leaf** (`sv_leaf_top.sv`, Mode B): the SV top directly
  instantiates the SystemC module in its module body (`inc_leaf u(.clock(clk),
  .din(din), .dout(dout))`). Ports and direction are discovered at the
  instantiation site; the bridge binds the SC module's own port signals and
  reuses the elaborator's existing port-connection assigns to propagate values
  to the actuals. The SystemC kernel is auto-stepped each time SV time
  advances, so no `$sc_step` is required.

## How the auto-derive works

### SC top (use-cases 1–3)

1. Run with `-s <module>` naming the SystemVerilog DUT and `--sc-source
   <testbench>.cc`.
2. xezim elaborates the SV DUT, reads its top-module ports (name, direction,
   width), and generates `XezTop`: a C++ `SC_MODULE` whose public `sc_signal`
   members mirror the SV ports, plus an `sc_start` interposer.
3. It compiles the testbench together with `XezTop` and `sc_shim.cc` into a
   `.so`, loads it, and runs the slice loop. The testbench just calls
   `sc_start()`; xezim drives the SV kernel per slice inside it.

### SV top (use-case 4)

1. Run with `-s <module>` naming the SystemVerilog top and `--sc-module <Name>`
   naming the pure-SystemC module to instantiate at the boundary (its ports
   must match the SV top's ports by name and width).
2. xezim elaborates the SV top, generates a model harness that instantiates
   `<Name>` and binds every crossing port through the registry, and compiles it
   with `<Name>`'s source and `sc_shim.cc`.
3. The SV top's `parameter`s are forwarded into the registry before the
   model ctor runs, so `sc_get_*_param` reads pick them up (`-G` overrides
   included).

## Parameter passing

Matching the reference `param_passing` examples:

* **SystemVerilog → SystemC** (use-case 4, *implemented*): the SV top's
  `parameter`s (elaborated values, after `-G` overrides) are forwarded into
  the co-sim registry and read by the SystemC model with `sc_get_int_param` /
  `sc_get_real_param` / `sc_get_string_param` / `sc_get_param`. `int`, `real`
  and `string` types are supported. Verified above.

* **SystemC → SystemVerilog** (an SC testbench overriding an SV module's
  parameters from its C++ main entrypoint): the infrastructure (the
  `xsc_set_param_*` registry + the SV-side `ParamOverride`/`-G` plumbing) is
  in place, but an SC-top run sets parameters *inside `sc_main`*, which runs
  *after* the SV worker has already elaborated the top. Making SC-set
  parameters affect the SV top requires a pre-elaboration rendezvous on the
  shared link — tracked as follow-up work (so use-case 4's SC reads this is
  the currently-exercised direction).

## Portability to reference simulators

The **SystemVerilog files in these examples are 100% portable** — they are
plain, vendor-independent RTL/module declarations with no SystemC knowledge.

The **SystemC testbenches (`*.cc`) are *not* drop-in runnable on a reference
simulator unchanged**, because the interop boundary is generated differently:

| Piece | xezim example | Reference simulator |
|-------|---------------|--------------------------------------|
| SV DUT | plain `module` | same, portable |
| SC wrapper type | `XezTop` (auto-generated, `sc_signal` members) | `sc_foreign_module` + a shell SV module with `(* foreign="SystemC" *)`, or `scgenmod`-generated ports |
| driving the run | `sc_start()` interposed onto the xezim slice driver | genuine `sc_start()` / `sc_main` kernel |
| SV↔SC binding | automatic by port name (no bind API) | vendor wrapper ports / `$sc_*` or export-import tasks |
| parameter passing | `sc_get_*_param` free functions (SV→SC) | same free functions (source-compatible, `vlog_to_sc`) |

So the examples show the **programming style** the reference flow uses (a
hand-written `SC_MODULE` boundary with `sc_in`/`sc_out`-style signal members,
plain `sc_main` calling `sc_start`), but to run the identical `.cc` under a
reference simulator you must re-target the wrapper class name and the
`sc_start` call to that vendor's API. The RTL, the port set, and the signal
types all carry over verbatim.

The SC-top examples here match the reference *semantics* (what the wrapper
exposes, how values/masters are inferred) — cross-verified in xezim against
the manual `xsc_cosim_run` flow, which produces identical results.