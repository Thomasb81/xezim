// Original example — pure SystemVerilog top (SV master) feeding parameter
// values into a pure SystemC model.
//
// This file is 100% portable SystemVerilog: no xsc_bind, no $sc_* tasks, no
// foreign-module syntax. xezim (like the reference simulators) treats the SV
// top's `parameter`s as a configuration surface and forwards them to the
// SystemC model that sits at the boundary, so the pure-SystemC module can
// shape its behaviour from SV-side values exactly the way a reference
// simulator's SystemVerilog->SystemC `param_passing/vlog_to_sc` example does.
//
//   SV top parameters        SystemC read        (reference: `vlog_to_sc`)
//   ----------------------   ------------------   ---------------------------
//   counter_size (int)  ->   sc_get_int_param("counter_size", &ok) / sc_get_param
//   width       (int)   ->   sc_get_int_param("width", &ok)
//   scale       (real)  ->   sc_get_real_param("scale", &ok)
//   tag         (string)->   sc_get_string_param("tag", &ok)
//
// Master is inferred as `sv`: the C++ TU defines no `sc_main` and no
// `xsc_model_build`; the pure SC_MODULE is picked up via `--sc-module`.
//
// Run:
//
//   SYSTEMC_HOME=<prefix> xezim --simulate -s sv_param_top \
//     -I <xezim>/src/sc_shim \
//     --sc-module sc_param_sink --sc-source sc_param_sink.cc \
//     sv_param_top.sv
//
// Override any parameter from the CLI (`-G<NAME>=<value>`, the same knob the
// reference example's SV-side `#(.name(value))` overrides map to):
//
//   ... --sc-module sc_param_sink ... -Gcounter_size=9 -Gscale=3.5 \
//       -Gtag="SHA256" sv_param_top.sv
//
// The SystemC model prints the values it received, proving the SV ->
// SystemC parameter path end to end.
module sv_param_top #(
  parameter int  counter_size = 4,
  parameter int  width        = 8,
  parameter real scale        = 2.5,
  parameter string tag         = "sq"
)(
  input  logic        clk,
  input  logic        rst_n,
  input  logic [7:0]  din,
  output logic [7:0]  dout,
  output logic        valid
);
  // Belt-and-braces: are the parameters actually elaborated to these values?
  // xezim forwards *the elaborated values* (after CLI -G overrides), so this
  // self-check and the SystemC read always agree.
  initial $display("SV_PARAM counter_size=%0d width=%0d scale=%0.1f tag=%s",
                   counter_size, width, scale, tag);
endmodule