// Reference example — pure SystemVerilog combinational DUT.
//
// No SystemC knowledge whatsoever. xezim derives the XezTop wrapper (public
// sc_signal members mirroring these ports) from the module declaration, so a
// plain SystemC testbench drives it with no bind API.
module sv_incr(
  input  logic [7:0]  din,
  output logic [7:0]  dout
);
  assign dout = din + 8'd1;
endmodule