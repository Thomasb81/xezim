// Original example — pure SystemC module reading SystemVerilog parameters
// (SV master -> SC, the reference `vlog_to_sc` parameter-passing pattern).
//
// A plain SC_MODULE with no bespoke xezim/`xsc_bind` API. Its ports mirror the
// SV top module's ports (`sv_param_top.sv`) by name; xezim instantiates it at
// the boundary via `--sc-module sc_param_sink` and connects each crossing
// port. The four `parameter`s of the SV top are forwarded into the co-sim
// parameter registry *before this module's ctor runs*, so the ctor can read
// them with the standard IEEE-1666-style free functions:
//
//     int   n = sc_get_int_param("counter_size", &ok);
//     double f = sc_get_real_param("scale", &ok);
//     const char* s = sc_get_string_param("tag", &ok);
//
// Build details: the SV->SC direction works out of the box because the SV top
// is elaborated first (the parameters are known statically and after any -G
// overrides). The SC->SV direction (an SC testbench overriding SV parameters
// from its C++ main entrypoint) needs the wrapper to run before
// elaboration — see the README's portability/limitations table.

#include <systemc>
#include <cstdint>
#include <cstdio>
#include "sc_api.h"

SC_MODULE(sc_param_sink) {
  sc_core::sc_in<bool> clk;
  sc_core::sc_in<bool> rst_n;
  sc_core::sc_in<std::uint64_t> din;
  sc_core::sc_out<std::uint64_t> dout;
  sc_core::sc_out<bool> valid;

  SC_CTOR(sc_param_sink) {
    int ok = 0;
    int cs = xsc_get_int_param("counter_size", -1, &ok);
    int wd = xsc_get_int_param("width", -1, &ok);
    double sc = xsc_get_real_param("scale", -1.0, &ok);
    const char* tag = xsc_get_string_param("tag", "", &ok);
    std::printf("SC_PARAM counter_size=%d width=%d scale=%.1f tag=%s\n",
                cs, wd, sc, tag);
  }
};