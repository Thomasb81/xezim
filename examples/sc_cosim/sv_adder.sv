// Reference example — pure SystemVerilog DUT.
//
// This is an ordinary .sv module with no SystemC knowledge at all. xezim
// auto-derives a C++ wrapper (`XezTop`) whose public `sc_signal` members
// mirror these ports, so a plain SystemC testbench can drive it with no
// bespoke bind API.
//
//   module ports         direction   width   SystemC exposure
//   -------------------  ----------  ------  ---------------------
//   din                  input       8  bits XezTop::din  (sc_in)
//   en                   input       1  bit  XezTop::en   (sc_in)
//   rst_n                input       1  bit  XezTop::rst_n (sc_in)
//   clk                  input       1  bit  XezTop::clk  (sc_in)
//   dout                 output      8  bits XezTop::dout (sc_out)
//   valid                output      1  bit  XezTop::valid(sc_out)
//
// Run from a SystemC testbench (SC top) with:
//
//   SYSTEMC_HOME=<prefix> xezim --simulate -s sv_adder \
//     -I <xezim>/src/sc_shim \
//     --sc-source sc_tb_sv_adder.cc \
//     sv_adder.sv
//
// The master is inferred as `sc` because the C++ TU defines sc_main. The
// wrapper is generated from these ports automatically — no --sc-map, no
// $sc_*, no xsc_bind in the testbench.
module sv_adder(
  input  logic        clk,
  input  logic        en,
  input  logic        rst_n,
  input  logic [7:0]  din,
  output logic [7:0]  dout,
  output logic        valid
);
  // Standard RTL register: posedge clock, async low reset.
  logic [7:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)      r <= 8'd0;
    else if (en)     r <= din + 8'd1;
  end
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)      valid <= 1'b0;
    else             valid <= en;
  end
  assign dout = r;
endmodule