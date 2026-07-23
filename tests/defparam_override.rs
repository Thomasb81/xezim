//! §23.10.1 — `defparam` hierarchical parameter override. xezim's parser used
//! to parse-and-discard `defparam`, so any parameter set that way silently kept
//! its default (root cause of a customer clock-mux staying on its bypass default
//! and never wiring the PLL clock through). Now parsed into a `Defparam` node
//! and applied during elaboration, including multi-level paths and generate-
//! block-scoped targets (`gblk.u.PARAM`).

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn passes(src: &str) -> bool {
    let out = line(src);
    out.iter().any(|m| m == "TEST_PASS") && !out.iter().any(|m| m == "TEST_FAIL")
}

#[test]
fn defparam_body_param() {
    // parameter declared in the module BODY, overridden via defparam.
    assert!(passes(
        r#"
module mux (output Z, input I0, input I1, input S);
  parameter SEL_POL = 0;
  assign Z = (S ^ SEL_POL) ? I1 : I0;
endmodule
module top;
  reg i0=0,i1=1,s=0; wire z;
  mux u (.Z(z), .I0(i0), .I1(i1), .S(s));
  defparam u.SEL_POL = 1;               // -> (0^1)=1 -> Z=I1=1
  initial begin #1; if (z===1'b1) $display("TEST_PASS"); else $display("TEST_FAIL"); end
endmodule
"#
    ));
}

#[test]
fn defparam_gates_a_generate() {
    // defparam controlling a generate branch (the clock-mux `_gen` shape).
    assert!(passes(
        r#"
module ckmux_gen (output Z, input I0, input I1, input S);
  parameter USE = 0;
  generate if (USE) assign Z = S ? I1 : I0; else assign Z = I0; endgenerate
endmodule
module top;
  reg i0=0,i1=1,s=1; wire z;
  ckmux_gen gmux (.Z(z), .I0(i0), .I1(i1), .S(s));
  defparam gmux.USE = 1;                // else branch drops I1; override picks the real mux
  initial begin #1; if (z===1'b1) $display("TEST_PASS"); else $display("TEST_FAIL"); end
endmodule
"#
    ));
}

#[test]
fn defparam_multilevel_path() {
    // `u.m.PARAM` — target a parameter of an instance nested one level down.
    assert!(passes(
        r#"
module mux (output Z, input I0, input I1, input S);
  parameter SEL_POL = 0; assign Z = (S ^ SEL_POL) ? I1 : I0;
endmodule
module wrap (output Z, input I0, input I1, input S); mux m(.Z(Z),.I0(I0),.I1(I1),.S(S)); endmodule
module top;
  reg i0=0,i1=1,s=0; wire z;
  wrap u (.Z(z), .I0(i0), .I1(i1), .S(s));
  defparam u.m.SEL_POL = 1;
  initial begin #1; if (z===1'b1) $display("TEST_PASS"); else $display("TEST_FAIL"); end
endmodule
"#
    ));
}

#[test]
fn defparam_through_generate_scope() {
    // `gblk.u.PARAM` — the instance is inside a generate block, so its flattened
    // name is `gblk.u`; the defparam path must match the dotted scope.
    assert!(passes(
        r#"
module leaf(output Z, input A); parameter INV=0; assign Z = INV?~A:A; endmodule
module top;
  reg a=1; wire z;
  generate if (1) begin : gblk
    leaf u(.Z(z), .A(a));
  end endgenerate
  defparam gblk.u.INV = 1;              // -> Z = ~1 = 0
  initial begin #1; if (z===1'b0) $display("TEST_PASS"); else $display("TEST_FAIL"); end
endmodule
"#
    ));
}

#[test]
fn defparam_multiple_assignments_last_wins() {
    // comma-separated + duplicate target (last write wins).
    assert!(passes(
        r#"
module m(output [W-1:0] Z, input [W-1:0] A); parameter W=1; parameter INV=0;
  assign Z = INV ? ~A : A;
endmodule
module top;
  reg [7:0] a=8'h0F; wire [7:0] z;
  m u(.Z(z), .A(a));
  defparam u.W = 8, u.INV = 1;          // widen to 8 and invert -> 0xF0
  defparam u.INV = 1;                   // duplicate; still 1
  initial begin #1; if (z===8'hF0) $display("TEST_PASS"); else $display("TEST_FAIL"); end
endmodule
"#
    ));
}
