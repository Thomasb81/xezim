//! Continuous assignments to one bit of an unpacked-array element,
//! `assign r[i][j] = ...`, where `r` is a 1-D unpacked array of packed
//! vectors.
//!
//! At the top level they drive the array. Inside an instantiated module they
//! are dropped: the array stays `z` and anything clocked through it never
//! fires. The generate-loop form is the shape of a PLIC's per-hart enable
//! clock gates, where xezim silently loses thousands of these assigns. The
//! `#[ignore]`d tests pin that divergence until it is fixed.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn top_level_element_bit_assigns_drive_the_array() {
    let o = out(r#"
module top;
  logic a = 1;
  wire [1:0] r [1:0];
  assign r[0][0] = a;
  assign r[0][1] = ~a;
  assign r[1][1] = a;
  assign r[1][0] = 1'b0;
  wire [3:0] p [1:0];
  assign p[1] = {a, a, 2'b01};
  initial begin
    #1 $display("T|r0=%b r1=%b p1=%b", r[0], r[1], p[1]);
    a = 0;
    #1 $display("T|r0=%b r1=%b p1=%b", r[0], r[1], p[1]);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        o,
        ["T|r0=01 r1=10 p1=1101", "T|r0=10 r1=00 p1=0001"],
        "{o:?}"
    );
}

#[test]
#[ignore = "known gap: element-bit assigns inside a sub-module are dropped"]
fn submodule_element_bit_assigns_drive_the_array() {
    let o = out(r#"
module direct (input logic a, output logic [1:0] r0, output logic [1:0] r1);
  wire [1:0] r [1:0];
  assign r[0][0] = a;
  assign r[0][1] = ~a;
  assign r[1][1] = a;
  assign r[1][0] = 1'b0;
  assign r0 = r[0];
  assign r1 = r[1];
endmodule
module top;
  logic a = 1;
  wire [1:0] r0, r1;
  direct dr (.a(a), .r0(r0), .r1(r1));
  initial begin
    #1 $display("T|r0=%b r1=%b", r0, r1);
    a = 0;
    #1 $display("T|r0=%b r1=%b", r0, r1);
    $finish;
  end
endmodule
"#);
    assert_eq!(o, ["T|r0=01 r1=10", "T|r0=10 r1=00"], "{o:?}");
}

#[test]
#[ignore = "known gap: element-bit assigns inside a sub-module are dropped"]
fn generate_loop_element_bit_clock_gates_fire_their_flops() {
    let o = out(r#"
module gclk (input logic clk_in, input logic en, output logic clk_out);
  assign clk_out = clk_in & en;
endmodule
module flop (input logic clk, input logic [3:0] d, output logic [3:0] q);
  initial q = 0;
  always @(posedge clk) q <= d;
endmodule
module busif (input logic clk, input logic [1:0] we0, input logic [1:0] we1,
              input logic [3:0] d, output logic [7:0] q0, output logic [7:0] q1,
              output logic [1:0] en0, output logic [1:0] en1);
  wire [1:0] wclk [1:0];
  wire [1:0] wen [1:0];
  wire [7:0] q [1:0];
  genvar i, j;
  for (i = 0; i < 2; i++) begin : G
    for (j = 0; j < 2; j++) begin : J
      assign wen[i][j] = (i == 0) ? we0[j] : we1[j];
      gclk g (.clk_in(clk), .en(wen[i][j]), .clk_out(wclk[i][j]));
      flop f (.clk(wclk[i][j]), .d(d), .q(q[i][4*j+:4]));
    end
  end
  assign q0 = q[0];
  assign q1 = q[1];
  assign en0 = wen[0];
  assign en1 = wen[1];
endmodule
module top;
  logic clk = 0;
  logic [1:0] we0 = 2'b01, we1 = 2'b10;
  logic [3:0] d = 4'h5;
  wire [7:0] q0, q1;
  wire [1:0] en0, en1;
  busif b (.clk(clk), .we0(we0), .we1(we1), .d(d), .q0(q0), .q1(q1), .en0(en0), .en1(en1));
  initial begin
    #1 clk = 1;
    #1 $display("T|en0=%b en1=%b q0=%h q1=%h", en0, en1, q0, q1);
    d = 4'ha; clk = 0; we0 = 2'b10; we1 = 2'b01;
    #1 clk = 1;
    #1 $display("T|en0=%b en1=%b q0=%h q1=%h", en0, en1, q0, q1);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        o,
        ["T|en0=01 en1=10 q0=05 q1=50", "T|en0=10 en1=01 q0=a5 q1=5a"],
        "{o:?}"
    );
}
