// Reference example — pure SystemC testbench for the SystemVerilog DUT in
// sv_adder.sv (SC top: SystemC owns the run, SystemVerilog is the DUT).
//
// No bespoke bind API: include the xezim-generated wrapper header
// (`#include "xez_top.h"`), instantiate XezTop, drive its public sc_signal
// members like any other SystemC signals, and call plain `sc_start()`.
// xezim interposes sc_start onto the co-simulation driver, so this file is
// an ordinary SystemC model end-to-end.
#include <systemc>
#include <cstdio>
#include "xez_top.h"

int sc_main(int, char**) {
  XezTop dut("dut");

  // Release reset, prime the register input.
  dut.rst_n.write(false);
  dut.en.write(false);
  dut.din.write(0);
  dut.clk.write(false);
  sc_start();

  dut.rst_n.write(true);
  dut.en.write(true);
  dut.din.write(41);
  dut.clk.write(false);
  sc_start();
  dut.clk.write(true);  sc_start();  // posedge: r <= din+1 = 42
  dut.clk.write(false); sc_start();

  uint64_t got = dut.dout.read();
  bool     ok   = dut.valid.read();
  std::printf("TAG_%s dout=%llu valid=%d\n",
              (got == 42) && ok ? "PASS" : "FAIL",
              (unsigned long long)got, (int)ok);
  return 0;
}