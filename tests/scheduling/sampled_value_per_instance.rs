//! §16.9.3 — sampled-value functions in PROCEDURAL code of child instances,
//! and the four-state, default-value and off-tick rules. Reference-validated.
//!
//! - A call site was keyed by its source span alone, so every instance of a
//!   child module shared the FIRST instance's inferred clock and history: a
//!   `$rose(x)` in an instance clocked by another net never fired (the
//!   lazy-children design counted 0 rises where the reference counts 2).
//!   Sites are now keyed by span and instance scope.
//! - An edge-controlled always block whose body blocks runs as a process and
//!   had no inferred clock at all.
//! - A tick before the first one compares against the operand's default
//!   sampled value (its declaration value), not "no change".
//! - `$fell` from x/z to 0 is a fall, and `$stable`/`$changed` compare with
//!   `===` (`2'bx1` to `2'b01` changed).
//! - Off its clock's tick (`$rose(x, @(posedge ck))` in a negedge block) the
//!   current sample is the operand's preponed value, as is the sample taken
//!   at a tick that a continuous assignment delays to a later delta of the
//!   slot (`.ck(a & en)`).

use xezim::simulate;

fn t_lines(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// Five instances of one checker on five different clocks (a vector bit, an
/// expression, the clock, an inverted clock through a wrapper), a generate
/// loop in the top, and a blocking always block.
#[test]
fn sampled_functions_per_instance() {
    const SRC: &str = r#"
module edg(input ck, input x, input [3:0] v);
  int rr = 0, ff = 0, ss = 0, cc = 0, pp = 0, p2 = 0, cr = 0;
  always @(posedge ck) begin
    if ($rose(x)) rr++;
    if ($fell(x)) ff++;
    if ($stable(v)) ss++;
    if ($changed(v)) cc++;
    pp += $past(v);
    p2 += $past(v, 2);
  end
  // explicit clocking form, evaluated from a non-clocked context
  always @(negedge ck) if ($rose(x, @(posedge ck))) cr++;
endmodule
module wrap(input ck, input x, input [3:0] v);
  edg e1(.ck(ck), .x(x), .v(v));
  edg e2(.ck(~ck), .x(x), .v(v));
endmodule
module tb;
  reg clk = 0; always #5 clk = ~clk;
  reg x = 0, a = 0, en = 0;
  reg [3:0] v = 0;
  reg [3:0] bus = 0;
  edg c1(.ck(bus[2]), .x(x), .v(v));
  edg c2(.ck(a & en), .x(x), .v(v));
  edg c3(.ck(clk), .x(x), .v(v));
  wrap w(.ck(clk), .x(x), .v(v));
  int grr[2];
  for (genvar i = 0; i < 2; i++) begin : g
    wire gck = i ? bus[1] : clk;
    always @(posedge gck) if ($rose(x)) grr[i]++;
  end
  int prr = 0;
  always @(posedge clk) begin
    if ($rose(x)) prr++;
    #1;
  end
  initial begin
    for (int t = 0; t < 16; t++) begin
      @(negedge clk);
      bus = bus + 1;
      a = t[0];
      en = (t > 3);
      x = (t == 2 || t == 3 || t == 7 || t == 9 || t == 12);
      v = (t * 3) % 5;
    end
    #1;
    $display("T|c1 %0d %0d %0d %0d %0d %0d %0d", c1.rr, c1.ff, c1.ss, c1.cc, c1.pp, c1.p2, c1.cr);
    $display("T|c2 %0d %0d %0d %0d %0d %0d %0d", c2.rr, c2.ff, c2.ss, c2.cc, c2.pp, c2.p2, c2.cr);
    $display("T|c3 %0d %0d %0d %0d %0d %0d %0d", c3.rr, c3.ff, c3.ss, c3.cc, c3.pp, c3.p2, c3.cr);
    $display("T|e1 %0d %0d %0d %0d %0d %0d %0d", w.e1.rr, w.e1.ff, w.e1.ss, w.e1.cc, w.e1.pp, w.e1.p2, w.e1.cr);
    $display("T|e2 %0d %0d %0d %0d %0d %0d %0d", w.e2.rr, w.e2.ff, w.e2.ss, w.e2.cc, w.e2.pp, w.e2.p2, w.e2.cr);
    $display("T|g %0d %0d prr %0d", grr[0], grr[1], prr);
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|c1 1 1 0 2 1 0 0",
            "T|c2 1 1 0 6 10 9 2",
            "T|c3 4 4 2 14 28 24 0",
            "T|e1 4 4 2 14 28 24 0",
            "T|e2 4 4 3 14 28 24 4",
            "T|g 4 1 prr 4",
        ],
    );
}
