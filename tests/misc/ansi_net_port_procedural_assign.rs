//! §23.2.2.3: an ANSI output port declared without a variable type (`output
//! b`, `output wire [3:0] q`, or a port inheriting from one) is a net, and a
//! net is not the target of a procedural assignment (§10.4); `output logic`,
//! `output reg`, `output int` and `output var` ports are variables. Matches
//! the reference simulator both ways.

use xezim::simulate;

#[test]
fn procedural_assignment_to_an_output_net_is_rejected() {
    for src in [
        "module top(input clk, input a, output b, output c);\n\
         always_ff @(posedge clk) begin b <= a; c <= a; end endmodule",
        "module top(input clk, output wire [3:0] q); initial q = 4'd1; endmodule",
        "module top(input clk, output [1:0] x, y); initial y = 2'd1; endmodule",
    ] {
        let e = simulate(src, 10)
            .err()
            .expect("procedural write to a net port accepted");
        assert!(e.contains("procedural assignment"), "{e}");
    }
}

#[test]
fn output_variable_ports_take_procedural_assignments() {
    let src = r#"
module sub(input clk, output logic q, output reg [3:0] r, output int n, output var v);
  initial begin q = 1; r = 4'd5; n = 7; v = 1; end
endmodule
module top;
  wire q, v; wire [3:0] r; int n; reg clk;
  sub s(.clk(clk), .q(q), .r(r), .n(n), .v(v));
  initial #1 $display("L7|%b %0d %0d %b", q, r, n, v);
endmodule
"#;
    let sim = simulate(src, 10).expect("variable output ports must run");
    assert!(
        sim.output.iter().any(|o| o.message == "L7|1 5 7 1"),
        "{:?}",
        sim.output
    );
}
