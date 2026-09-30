//! §6.6 / §23.2.2.3: an output port with no data type is a net. When nothing
//! drives it, the port and the parent net it connects to float at `z`.
//!
//! xezim starts such a port at `x`, whether the port is ANSI, non-ANSI or
//! a vector. The `#[ignore]`d test pins that divergence until it is fixed.
//! Expected values are the reference simulator's.

#[test]
#[ignore = "known gap: an undriven implicit-net output starts at x, not z"]
fn undriven_implicit_net_outputs_float_at_z() {
    let sim = xezim::simulate(
        r#"
module c1 (output o);
endmodule
module c2 (o);
  output o;
endmodule
module c3 (output [1:0] o);
endmodule
module top;
  wire w1, w2;
  wire [1:0] w3;
  c1 u1 (.o(w1));
  c2 u2 (.o(w2));
  c3 u3 (.o(w3));
  initial #1 $display("T|w1=%b w2=%b w3=%b u1.o=%b", w1, w2, w3, u1.o);
endmodule
"#,
        100,
    )
    .expect("simulate");
    let o: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(o, ["T|w1=z w2=z w3=zz u1.o=z"], "{o:?}");
}
