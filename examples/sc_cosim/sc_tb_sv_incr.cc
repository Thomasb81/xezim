// Reference example — pure SystemC testbench for the SystemVerilog
// combinational DUT in sv_incr.sv (SC top).
//
// The seamless auto-derive flow: include the generated wrapper, instantiate
// XezTop, drive .din, read .dout — no xsc_bind, no --sc-map, no $sc_*.
#include <systemc>
#include <cstdio>
#include "xez_top.h"

int sc_main(int, char**) {
  XezTop dut("dut");
  dut.din.write(41);
  sc_start();   // interposed onto the co-simulation driver
  uint64_t q = dut.dout.read();
  std::printf("TAG_%s dout=%llu\n", (q == 42) ? "PASS" : "FAIL",
              (unsigned long long)q);
  return 0;
}