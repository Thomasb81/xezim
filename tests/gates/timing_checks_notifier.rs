//! §31 — `$setup`, `$hold` and `$width` in a specify block, with a notifier
//! that drives the flop output to x on any violation. The expected values
//! are the reference simulator's: the first capture is clean (q=0), the
//! setup violation at 20 ns and the hold violation at 29.6 ns each toggle
//! the notifier, so q reads x at both later samples (the reference also
//! prints one timing-violation error per check).
//!
//! Timing checks are not modelled today (`+notimingcheck` is documented as
//! a no-op), so the notifier never toggles and q keeps its captured values.

use xezim::simulate;

#[test]
#[ignore = "specify timing checks ($setup/$hold/$width + notifier) are not modelled (fix pending)"]
fn setup_hold_width_violations_toggle_the_notifier() {
    let sim = simulate(
        r#"
`timescale 1ns/1ps
module dff (input d, input clk, output reg q);
  reg notifier = 0;
  always @(posedge clk) q <= d;
  always @(notifier) q <= 1'bx;
  specify
    (clk => q) = (0.5, 0.6);
    $setup(d, posedge clk, 2.0, notifier);
    $hold(posedge clk, d, 1.0, notifier);
    $width(posedge clk, 3.0);
  endspecify
endmodule
module tb;
  reg d = 0, clk = 0; wire q;
  dff u (.d(d), .clk(clk), .q(q));
  initial begin
    #10 clk = 1; #1 $display("T1 q=%b", q);
    #4 clk = 0;
    #4.5 d = 1; #0.5 clk = 1;
    #1 $display("T2 q=%b", q);
    #0.2 d = 0;
    #3 clk = 0;
    #5 clk = 1; #0.4 d = 1;
    #2 $display("T3 q=%b", q);
    #1 clk = 0; #1 clk = 1; #1 clk = 0;
    #5 $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("simulate failed");
    let o: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in ["T1 q=0", "T2 q=x", "T3 q=x"] {
        assert!(o.iter().any(|l| l.contains(want)), "missing `{want}`: {o:?}");
    }
}
