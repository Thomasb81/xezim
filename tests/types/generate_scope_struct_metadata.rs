//! §27.4 — packed-struct metadata for declarations inside GENERATE scopes.
//! Reference-validated (widths AND element-wise values — widths alone lie).
//!
//! A `burst_t [0:0][1:0] sig;` inside `if (1) begin : g` elaborated at the
//! right total width, but `$bits(g.sig[0][0])` read 1 and every member select
//! returned garbage: the elaborate_items DataDeclaration arm (which generate
//! branches route through) registered widths and generic dims but never the
//! struct layout, the typedef-array shape, or the declared type — all of
//! which the identical module-scope declaration got.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package P;
  typedef struct packed {
    logic [1:0][63:0] lanes;
    logic [1:0][7:0]  mask;
    logic [1:0]       en;
  } burst_t;   // 146
endpackage
module tb;
  import P::*;
  generate
    if (1) begin : g
      burst_t [0:0][1:0] sig;
      logic [31:0] w_all, w_elem, w_memb;
      assign w_all  = $bits(sig);
      assign w_elem = $bits(sig[0][0]);
      assign w_memb = $bits(sig[0][0].lanes[0]);
      logic [63:0] lane00, lane11;
      logic [7:0]  m10;
      assign lane00 = sig[0][0].lanes[0];
      assign lane11 = sig[0][1].lanes[1];
      assign m10    = sig[0][1].mask[0];
    end
  endgenerate
  // The test harness's get_signal cannot resolve generate-scoped names, so
  // mirror everything into TOP-scope signals (same convention as the existing
  // generate tests).
  logic [31:0] t_w_all, t_w_elem, t_w_memb;
  logic [63:0] t_lane00, t_lane11;
  logic [7:0]  t_m10;
  initial begin
    g.sig[0][0] = {64'hAAAA_AAAA_AAAA_AAA1, 64'hBBBB_BBBB_BBBB_BBB0, 8'hC1, 8'hC0, 2'b10};
    g.sig[0][1] = {64'hDDDD_DDDD_DDDD_DDD1, 64'hEEEE_EEEE_EEEE_EEE0, 8'hF1, 8'hF0, 2'b01};
    #1;
    // Procedural mirrors: hierarchical generate-scope reads resolve on the
    // procedural path (continuous-assign sources from generate scopes are a
    // separate, pre-existing gap).
    t_w_all  = g.w_all;
    t_w_elem = g.w_elem;
    t_w_memb = g.w_memb;
    t_lane00 = g.lane00;
    t_lane11 = g.lane11;
    t_m10    = g.m10;
  end
endmodule
"#;

#[test]
fn generate_if_scope_struct_widths() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "t_w_all"), 292);
    assert_eq!(u(&sim, "t_w_elem"), 146, "was 1: no element metadata in generate scopes");
    assert_eq!(u(&sim, "t_w_memb"), 64, "was 1: no member strides in generate scopes");
}

#[test]
fn generate_if_scope_struct_values_flow() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "t_lane00"), 0xBBBB_BBBB_BBBB_BBB0, "lanes[0] is the LOW lane");
    assert_eq!(u(&sim, "t_lane11"), 0xDDDD_DDDD_DDDD_DDD1);
    assert_eq!(u(&sim, "t_m10"), 0xF0);
}

/// KNOWN GAP, pinned: hierarchical access to FOR-generate block signals
/// (`gl[i].x`) resolves to nothing — reads return a 32-bit zero and writes
/// vanish, for PLAIN logic as much as for struct types (the reference reads
/// p0=11 p1=22 d0=22 d1=44 here). Only localparams get the `label[i].name`
/// alias today. Needs a signal-aliasing scheme through resolve/read/write
/// plus metadata aliasing — tracked separately from the metadata fix above.
#[test]
#[ignore = "for-generate hierarchical signal access is not implemented (label[i].name aliases exist only for localparams)"]
fn for_generate_hierarchical_signal_access() {
    let src = r#"
module tb;
  generate
    for (genvar i = 0; i < 2; i++) begin : gl
      logic [7:0] plain;
      logic [7:0] doubled;
      assign doubled = plain * 2;
    end
  endgenerate
  initial begin
    gl[0].plain = 8'h11;
    gl[1].plain = 8'h22;
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("gl[0].doubled"), 0x22);
    assert_eq!(g("gl[1].doubled"), 0x44);
}
