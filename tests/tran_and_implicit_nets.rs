//! Two defects found while debugging a bidirectional DDR bus testbench.
//!
//! 1. §28.8 `tran` / `tranif0` / `tranif1` were modelled as a ONE-directional
//!    `assign terminal0 = terminal1`, so the second terminal was never driven,
//!    a disabled switch's `z` erased the net's own driver, and contention
//!    between two drivers resolved to whichever wrote last rather than to `x`.
//!    Each switch now bridges its terminals' OWN drivers with the wired-net
//!    resolution of Table 28-1.
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

/// §28.8 compliance: the resolution table, the conditional switches, and the
/// unknown-control case.
const SWITCHES: &str = r#"
module tb;
  logic ctrl;
  wire net_a, net_b, net_c, net_d, net_e, net_f;
  logic val_a, val_b, val_c, val_d, val_e, val_f;

  assign net_a = val_a;
  assign net_b = val_b;
  assign net_c = val_c;
  assign net_d = val_d;
  assign net_e = val_e;
  assign net_f = val_f;

  tran    u_tran    (net_a, net_b);
  tranif1 u_tranif1 (net_c, net_d, ctrl);
  tranif0 u_tranif0 (net_e, net_f, ctrl);

  int fails;
  initial begin
    fails = 0;
    #1;
    // tran: z yields to a driven value, in both directions.
    val_a = 1'b1; val_b = 1'bz; #10;
    if (net_a !== 1'b1 || net_b !== 1'b1) fails++;
    val_a = 1'bz; val_b = 1'b0; #10;
    if (net_a !== 1'b0 || net_b !== 1'b0) fails++;
    // tran: contention gives x on both nets.
    val_a = 1'b1; val_b = 1'b0; #10;
    if (net_a !== 1'bx || net_b !== 1'bx) fails++;

    // tranif1 disabled: each net keeps its own driver.
    ctrl = 1'b0; val_c = 1'b1; val_d = 1'b0; #10;
    if (net_c !== 1'b1 || net_d !== 1'b0) fails++;
    // enabled: contention -> x
    ctrl = 1'b1; #10;
    if (net_c !== 1'bx || net_d !== 1'bx) fails++;
    // enabled: z passes through
    val_c = 1'bz; val_d = 1'b1; #10;
    if (net_c !== 1'b1 || net_d !== 1'b1) fails++;

    // tranif0 has the opposite polarity.
    ctrl = 1'b1; val_e = 1'b0; val_f = 1'b1; #10;
    if (net_e !== 1'b0 || net_f !== 1'b1) fails++;
    ctrl = 1'b0; #10;
    if (net_e !== 1'bx || net_f !== 1'bx) fails++;

    // An unknown control makes differing bits unknown.
    ctrl = 1'bx; val_c = 1'b1; val_d = 1'b0; #10;
    if (net_c !== 1'bx || net_d !== 1'bx) fails++;
  end
endmodule
"#;

#[test]
fn bidirectional_switches_follow_the_resolution_table() {
    let sim = simulate(SWITCHES, 500).expect("simulate failed");
    let fails = sim
        .get_signal("fails")
        .or_else(|| sim.get_signal("tb.fails"))
        .expect("fails")
        .to_u64()
        .unwrap_or(99);
    assert_eq!(fails, 0, "{} of the 9 IEEE 1800-2017 §28.8 checks failed", fails);
}
