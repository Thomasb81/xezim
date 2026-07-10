//! Two defects found while debugging a bidirectional DDR bus testbench.
//!
//! 1. §28.4 `tran` / `rtran` were modelled as a ONE-directional connection
//!    (`assign terminal0 = terminal1`), so the second terminal was never
//!    driven. A bidirectional switch is modelled here as a pair of opposing
//!    continuous assigns: a high-impedance driver does not overwrite the far
//!    side, so the settle loop converges to the resolved value on both nets.
//!
//! 2. §6.10 implicit-net creation descended into a `MemberAccess` base, so a
//!    cross-module reference (`testbench.chip.dqs`) had its ROOT — the instance
//!    name — declared as a stray 1-bit net under the current prefix. That net
//!    then drove the real one to X.

use xezim::simulate;

/// A tri-state driver on one net must appear on the other side of the `tran`,
/// and high-impedance must stay high-impedance.
const TRAN: &str = r#"
module tb;
  wire [3:0] a, b;
  logic en;
  logic [3:0] d;
  assign a = en ? d : 4'bzzzz;
  tran t (a, b);

  logic [3:0] b_off, a_on, b_on;
  logic off_is_z;
  initial begin
    en = 0; d = 4'h0;
    #1 b_off = b;
    off_is_z = (b === 4'bzzzz);
    en = 1; d = 4'hA;
    #1 a_on = a;
       b_on = b;
  end
endmodule
"#;

/// A hierarchical reference in a sub-module must not declare its root as a net.
const XMR: &str = r#"
module leaf ();
  wire [3:0] v;
  assign v = 4'hA;
endmodule
module probe ();
  wire [3:0] seen;
  assign seen = top.l_inst.v;   // cross-module READ
endmodule
module top;
  leaf  l_inst();
  probe p_inst();
  logic [3:0] observed;
  initial #1 observed = p_inst.seen;
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or(0)
        & 0xF
}

#[test]
fn tran_propagates_in_both_directions() {
    let sim = simulate(TRAN, 100).expect("simulate failed");
    // The driven side and the far side agree.
    assert_eq!(u(&sim, "a_on"), 0xA);
    assert_eq!(u(&sim, "b_on"), 0xA, "tran never drove its second terminal");
}

#[test]
fn tran_keeps_high_impedance_when_undriven() {
    let sim = simulate(TRAN, 100).expect("simulate failed");
    assert_eq!(u(&sim, "off_is_z"), 1, "an undriven tran net must stay z");
}

#[test]
fn a_cross_module_reference_does_not_declare_its_root_as_a_net() {
    let sim = simulate(XMR, 100).expect("simulate failed");
    assert_eq!(u(&sim, "observed"), 0xA, "the cross-module read was clobbered");
}
