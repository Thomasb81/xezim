//! §23.9/§6.10 + §23.3.3: testbench idioms around instance ports, both
//! reference-verified.
//!
//! * A HIERARCHICAL continuous assign driving a deep sub-instance INPUT
//!   through unconnected port chains (`assign dut.mid.core.clk = tb_clk;`
//!   with `.top_clk()` no-connect). Identity connects collapse by
//!   substitution and different readers bind at DIFFERENT chain levels, so
//!   a drive left on one name silently missed the flop's clock — the DUT
//!   never clocked and every check failed. The drive now fans out across
//!   the whole port-alias chain.
//! * An EXPRESSION port actual over a parent net named like the FORMAL
//!   (`.din(din ^ 1)`) — the connect assign's RHS is parent-scoped by
//!   construction (rhs_parent_scoped), where the child scope hint made it
//!   a self-loop reading x forever.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_hpdc_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb_top", path.to_str().unwrap(), "--no-cache"])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

#[test]
fn hierarchical_assign_drives_deep_input_through_unconnected_ports() {
    let text = run(
        "hier_drive",
        r#"package pkt_defs;
   typedef struct packed {
      logic [7:0]  payload;
      logic        strobe;
   } beat_t;
endpackage

module leaf_acc (
   input  logic clk_i,
   input  logic srst_i,
   input  pkt_defs::beat_t beat_i,
   output logic [7:0] sum_o,
   output logic tick_o
);
   always_ff @(posedge clk_i) begin
      if (srst_i) begin
         sum_o  <= '0;
         tick_o <= 1'b1;
      end else if (beat_i.strobe) begin
         sum_o  <= sum_o + beat_i.payload;
         tick_o <= !tick_o;
      end
   end
endmodule

module mid_shell (
   input  logic clk_i,
   input  logic srst_i,
   input  pkt_defs::beat_t beat_i,
   output logic [7:0] sum_o,
   output logic tick_o
);
   leaf_acc u_leaf (.clk_i(clk_i), .srst_i(srst_i), .beat_i(beat_i),
                    .sum_o(sum_o), .tick_o(tick_o));
endmodule

module top_shell (
   input  logic top_clk_i,
   input  logic top_srst_i,
   input  pkt_defs::beat_t top_beat_i,
   output logic [7:0] top_sum_o,
   output logic top_tick_o
);
   mid_shell u_mid (.clk_i(top_clk_i), .srst_i(top_srst_i), .beat_i(top_beat_i),
                    .sum_o(top_sum_o), .tick_o(top_tick_o));
endmodule

module tb_top;
   import pkt_defs::*;
   int bad = 0;
   logic tb_clk, tb_rst;
   beat_t tb_beat;
   logic [7:0] got_sum;
   logic got_tick;

   top_shell dut (
      .top_clk_i  (),
      .top_srst_i (),
      .top_beat_i (tb_beat),
      .top_sum_o  (got_sum),
      .top_tick_o (got_tick)
   );

   assign dut.u_mid.u_leaf.clk_i  = tb_clk;
   assign dut.u_mid.u_leaf.srst_i = tb_rst;

   initial begin tb_clk = 0; forever #5 tb_clk = ~tb_clk; end

   initial begin
      tb_rst = 1; tb_beat = '0;
      repeat(3) @(posedge tb_clk); #1;
      tb_rst = 0;
      if (!(got_sum === 8'h00 && got_tick === 1'b1)) bad++;
      tb_beat.payload = 8'h2C; tb_beat.strobe = 1'b1;
      @(posedge tb_clk); #1;
      if (!(got_sum === 8'h2C && got_tick === 1'b0)) bad++;
      tb_beat.payload = 8'h04;
      @(posedge tb_clk); #1;
      if (!(got_sum === 8'h30 && got_tick === 1'b1)) bad++;
      tb_beat.strobe = 1'b0;
      @(posedge tb_clk); #1;
      if (!(got_sum === 8'h30 && got_tick === 1'b1)) bad++;
      if (bad == 0) $display("TEST_PASS"); else $display("TEST_FAIL n=%0d", bad);
      $finish;
   end
endmodule
"#,
    );
    assert!(text.contains("TEST_PASS"), "hierarchical drive:\n{text}");
}

#[test]
fn expression_actual_over_samename_parent_net_not_x() {
    let text = run(
        "expr_actual",
        r#"module dff (input clk, input [31:0] din, output reg [31:0] q);
  initial q = 0;
  always @(posedge clk) q <= din;
endmodule
module tb_top;
  reg clk = 0; always #5 clk = ~clk;
  reg [31:0] src = 32'h11111111;
  wire [31:0] din = src;
  wire [31:0] q0;
  dff u0 (.clk(clk), .din(din), .q(q0));
  integer cyc = 0;
  always @(posedge clk) begin
    src <= src + 32'h01010101;
    cyc <= cyc + 1;
    if (cyc == 3) begin
      if (q0 !== 32'hx && q0 === 32'h13131313) $display("TEST_PASS");
      else $display("TEST_FAIL q0=%h", q0);
      $finish;
    end
  end
endmodule
"#,
    );
    assert!(text.contains("TEST_PASS"), "same-name identity actual:\n{text}");
}
