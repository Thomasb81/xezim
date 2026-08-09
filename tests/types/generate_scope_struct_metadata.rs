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

/// §27.6 — hierarchical access to FOR-generate block signals. Named blocks'
/// declarations now take their LRM hierarchical name (`gl[1].plain`) — the
/// dotted-flat-key convention named IF-generate blocks always used — instead
/// of an opaque `x__gf_...` rename that nothing outside the block could
/// address: reads returned a 32-bit zero and writes VANISHED, for plain
/// logic as much as struct types. Reference-validated (p0=11 p1=22 d0=22
/// d1=44, and the nested case 10/21/40/51).
#[test]
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
  logic [7:0] t_d0, t_d1;
  initial begin
    gl[0].plain = 8'h11;
    gl[1].plain = 8'h22;
    #1;
    t_d0 = gl[0].doubled;
    t_d1 = gl[1].doubled;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("t_d0"), 0x22, "read through gl[0]. — was 0, writes vanished");
    assert_eq!(g("t_d1"), 0x44);
}

/// Nested for-generate: the inner scope inserts BEFORE the base name
/// (`outer[0].inner[1].q`), matching the LRM path — a naive prefix would
/// have produced `inner[1].outer[0].q`.
#[test]
fn nested_for_generate_hierarchical_access() {
    let src = r#"
module tb;
  generate
    for (genvar i = 0; i < 2; i++) begin : outer
      for (genvar j = 0; j < 2; j++) begin : inner
        logic [7:0] q;
        logic [7:0] twice;
        assign twice = q + 8'(i*16 + j);
      end
    end
  endgenerate
  logic [7:0] t00, t01, t10, t11;
  initial begin
    outer[0].inner[0].q = 8'h10;
    outer[0].inner[1].q = 8'h20;
    outer[1].inner[0].q = 8'h30;
    outer[1].inner[1].q = 8'h40;
    #1;
    t00 = outer[0].inner[0].twice;
    t01 = outer[0].inner[1].twice;
    t10 = outer[1].inner[0].twice;
    t11 = outer[1].inner[1].twice;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("t00"), 0x10);
    assert_eq!(g("t01"), 0x21);
    assert_eq!(g("t10"), 0x40);
    assert_eq!(g("t11"), 0x51);
}
