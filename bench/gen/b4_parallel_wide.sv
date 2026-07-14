// B4 parallel-scaling: 128 independent units
module unit #(parameter int ID = 0) (input logic clk, output logic [31:0] out);
  logic [31:0] a, b, c;
  always_ff @(posedge clk) begin
    a <= a + 32'd1 + ID;
    b <= b ^ (a << 3);
    c <= c + (b >> 2) + a;
  end
  assign out = c;
endmodule
module bench_parallel;
  bit clk = 0;
  logic [31:0] outs [128];
  int cyc = 0;
  always #1 clk = ~clk;
  genvar g;
  generate
    for (g = 0; g < 128; g++) begin : u
      unit #(.ID(g)) inst (.clk(clk), .out(outs[g]));
    end
  endgenerate
  always_ff @(posedge clk) cyc <= cyc + 1;
  initial begin
    #(200000);
    $display("BENCH_DONE cycles=%0d checksum=%0d", cyc, outs[0] + outs[127]);
    $finish;
  end
endmodule
