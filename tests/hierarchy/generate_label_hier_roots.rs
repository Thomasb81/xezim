//! §27.6: a generate block's label names a scope and may root a hierarchical
//! reference — `blk.f(0)` to a function declared in the selected branch
//! (ivtest `pr2350934`, `pr2350934b`, `pr2350988`), `foo[i-1].t` to an
//! earlier loop iteration (yosys `simple_generate`). The check for names
//! declared nowhere in the design, run over INSTANTIATED module bodies, did
//! not count generate labels as declarations and rejected these designs
//! ("Undeclared identifier 'block'"). Cross-checked against the reference
//! simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 10)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn function_in_labelled_generate_branch_of_a_child() {
    let o = lines(
        r#"
module test ();
  parameter param = 3;
  reg [2:0] dummy;
  initial dummy = block.f(0);
  generate
    if (param == 1) begin : block
      function [2:0] f; input i; f = param; endfunction
    end else if (param == 2) begin : block
      function [2:0] f; input i; f = param + 3; endfunction
    end
  endgenerate
endmodule
module chain #(parameter N = 3) (input [N-1:0] a, output [N-1:0] b);
  genvar i;
  for (i = 0; i < N; i = i + 1) begin : foo
    localparam PREV = i - 1;
    wire t;
    if (i == 0) assign t = a[0];
    else assign t = foo[PREV].t & a[i];
    assign b[i] = t;
  end
endmodule
module top ();
  test #(1) a();
  test #(2) b();
  reg [2:0] in = 3'b111;
  wire [2:0] out;
  chain c(.a(in), .b(out));
  initial #1 $display("G|%0d %0d %b", a.dummy, b.dummy, out);
endmodule
"#,
    );
    // Elaborates, and the loop-carried chain reads the earlier iteration.
    assert!(
        o.iter().any(|l| l.starts_with("G|") && l.ends_with(" 111")),
        "{o:?}"
    );
}
