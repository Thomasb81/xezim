//! §10.3.1 / §6.7.1: a delay on a net declaration applies to every driver
//! of the net, not just to a declaration assignment.
//!
//! `wire #2 wi = a;` is delayed correctly. `wire #2 w;` followed by a separate
//! `assign w = a;` ignores the delay, at the top level and in a sub-module
//! alike. The `#[ignore]`d test pins that divergence until it is fixed.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn net_delay_with_a_declaration_assignment() {
    let o = out(r#"
module top;
  logic a = 0;
  wire #2 wi = a;
  initial begin
    #10 a = 1;
    #1 $display("T|t=%0t wi=%b", $time, wi);
    #1 $display("T|t=%0t wi=%b", $time, wi);
    $finish;
  end
endmodule
"#);
    assert_eq!(o, ["T|t=11 wi=0", "T|t=12 wi=1"], "{o:?}");
}

#[test]
#[ignore = "known gap: a net delay without a declaration assignment is ignored"]
fn net_delay_applies_to_a_separate_continuous_assign() {
    let o = out(r#"
module sub (input logic a, output wire y);
  wire #3 w;
  assign w = a;
  assign y = w;
endmodule
module top;
  logic a = 0;
  wire #2 w;
  assign w = a;
  wire ys;
  sub s (.a(a), .y(ys));
  initial begin
    #10 a = 1;
    #1 $display("T|t=%0t w=%b ys=%b", $time, w, ys);
    #1 $display("T|t=%0t w=%b ys=%b", $time, w, ys);
    #1 $display("T|t=%0t w=%b ys=%b", $time, w, ys);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        o,
        ["T|t=11 w=0 ys=0", "T|t=12 w=1 ys=0", "T|t=13 w=1 ys=1"],
        "{o:?}"
    );
}
