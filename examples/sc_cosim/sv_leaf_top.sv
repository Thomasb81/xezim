// Mode B: a pure-SystemVerilog netlist that DIRECTLY instantiates a pure
// SystemC module. There is no SV `module inc_leaf` anywhere — xezim reflects
// the SystemC module's ports from inc_leaf.cc, synthesizes a placeholder for
// elaboration, and auto-steps the SystemC kernel at each SV time advance, so
// this testbench needs NO $sc_step tasks. This is 100% portable SV.
module sv_leaf_top;
  logic clk = 0;
  logic [7:0] din = 8'h2a;   // 42
  logic [7:0] dout;

  always #5 clk = ~clk;
  initial begin
    #30 $display("SV_LEAF dout=%0d (expect 43)", dout);
    #2 $finish;
  end

  // The SystemC leaf registers din+1 on the posedge of its own clock.
  inc_leaf u(.clock(clk), .din(din), .dout(dout));
endmodule