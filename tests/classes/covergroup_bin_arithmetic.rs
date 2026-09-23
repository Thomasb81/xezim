//! §19.11 — coverage computation over explicit, array, illegal and ignore
//! bins and a cross. The expected numbers are both the reference
//! simulator's and a hand count: `cp_op` has 4 bins (`low` + `high[4..6]`;
//! value 7 is illegal) and all are hit; `cp_mode` has 3 bins (value 3
//! ignored), all hit; the cross has 12 bins, 11 hit (`high[6] x 2` never
//! occurs) = 91.67%; the group is the mean of the three, 97.22%.
//!
//! xezim reports 72.50 for the group and for every coverpoint and the
//! cross today (a per-item query returns the group value).

use xezim::simulate;

#[test]
#[ignore = "covergroup/coverpoint/cross get_coverage values are wrong (fix pending)"]
fn explicit_array_illegal_and_ignore_bins_with_cross() {
    let sim = simulate(
        r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [2:0] op; logic [1:0] mode;
  covergroup cg @(posedge clk);
    option.per_instance = 1;
    cp_op: coverpoint op { bins low = {[0:3]}; bins high[] = {[4:7]}; illegal_bins bad = {7}; }
    cp_mode: coverpoint mode { ignore_bins ig = {3}; }
    x: cross cp_op, cp_mode;
  endgroup
  cg c = new;
  initial begin
    op = 0; mode = 0;
    repeat (20) @(negedge clk) begin op = (op + 1) % 7; mode = (mode + 1) % 3; end
    $display("COV cg=%0.2f op=%0.2f mode=%0.2f x=%0.2f", c.get_coverage(), c.cp_op.get_coverage(),
             c.cp_mode.get_coverage(), c.x.get_coverage());
    $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("simulate failed");
    let o: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(o.iter().any(|l| l.contains("COV cg=97.22 op=100.00 mode=100.00 x=91.67")), "{o:?}");
}
