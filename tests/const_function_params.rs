//! IEEE 1800-2017 §13.4.3 constant functions in parameter initializers.
//!
//! `parameter LOG_X = log2(X)` used to be DEFERRED with value 0 during
//! elaboration (function calls couldn't be const-evaluated), so every
//! dependent parameter wrapped (`LOG_X - 6` → 4294967290) and dependent port
//! widths collapsed — on a customer gate-level design this destroyed 55-bit
//! request buses (elaborated as 1-bit) and the memory-read flow with them.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn log2_style_param_resolves_at_elaboration() {
    // The classic vendor ceil-log2: for-loop, assignment to the function
    // name, and an increment in the for-step.
    let sim = simulate(
        r#"
module sub #(
  parameter SIZE = 64,
  parameter LOG_SIZE = clog2f(SIZE),
  parameter LOG_SIZE_64B = LOG_SIZE - 6,
  parameter REQ_W = 55
) (
  input  [REQ_W-1:0] req,
  output [LOG_SIZE-1:0] cnt
);
  function integer clog2f(input integer value);
    integer v;
    begin
      v = value - 1;
      for (clog2f = 0; v > 0; clog2f = clog2f + 1)
        v = v >> 1;
    end
  endfunction
  assign cnt = req[LOG_SIZE-1:0];
  initial $display("W=%0d,%0d,%0d,%0d", LOG_SIZE, LOG_SIZE_64B, $bits(req), $bits(cnt));
endmodule
module top;
  wire [54:0] req = 55'h5A;
  wire [5:0] cnt;
  sub u (.req(req), .cnt(cnt));
  initial #1 $finish;
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "W=6,0,55,6"),
        "const-fn param sizing wrong; output: {:?}",
        msgs
    );
}

#[test]
fn recursive_and_if_based_const_fns() {
    // if/else + recursion + a localparam consumer in the TOP module.
    let sim = simulate(
        r#"
module top;
  function integer rlog2(input integer n);
    if (n <= 1) rlog2 = 0;
    else rlog2 = 1 + rlog2((n + 1) / 2);
  endfunction
  localparam W = rlog2(1024);
  localparam W2 = rlog2(1000);
  reg [W-1:0] bus;
  initial begin
    $display("R=%0d,%0d,%0d", W, W2, $bits(bus));
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "R=10,10,10"),
        "recursive const fn wrong; output: {:?}",
        msgs
    );
}

#[test]
fn package_const_fn_param_via_wildcard_import() {
    let sim = simulate(
        r#"
package szpkg;
  function automatic integer plog2(input integer value);
    integer v;
    begin
      v = value - 1;
      for (plog2 = 0; v > 0; plog2 = plog2 + 1) v = v >> 1;
    end
  endfunction
  parameter DEPTH = 128;
  parameter AW = plog2(DEPTH);
endpackage
module top;
  import szpkg::*;
  reg [AW-1:0] addr;
  initial begin $display("PKG=%0d,%0d", AW, $bits(addr)); $finish; end
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "PKG=7,7"),
        "package const-fn param wrong; output: {:?}",
        msgs
    );
}
