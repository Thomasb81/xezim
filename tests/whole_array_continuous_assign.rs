//! §10.6.2 — a whole-ARRAY continuous assignment between unpacked arrays.
//!
//! ```systemverilog
//! logic [9:0] src [4];
//! logic [9:0] dst [4];
//! assign dst = src;
//! ```
//!
//! An unpacked array has no single backing signal — its ELEMENTS are the
//! signals — so this was pushed as one scalar assignment that matched no
//! target and did nothing at all: `dst` stayed x for the whole run and never
//! responded to a change in `src`. The per-element spelling
//! (`assign dst[i] = src[i];`) always worked, as did procedural writes and the
//! packed-2D form (`logic [3:0][9:0]`), which is what made this so quiet — the
//! shape looks ordinary and only the whole-array spelling fails.
//!
//! Found while debugging a CDC design where an array output port driven this
//! way left the parent's array x, which in turn made four downstream
//! struct-member assigns propagate x. The struct assigns looked like the
//! culprit; they were faithfully forwarding an x source.
//!
//! KNOWN GAP (not covered here, deliberately): the same assignment INSIDE an
//! instantiated sub-module is still dropped — inlined bodies reach neither
//! pending-drain this fix hooks. `scratchpad/sm/s4.sv` / `s6.sv` reproduce it.
//!
//! Arrays of DIFFERENT element counts are left on their existing path rather
//! than expanded (assigning between differently-sized unpacked arrays is
//! illegal per §10.9 anyway, so there is nothing useful to pin about it).
//!
//! Expectations below are byte-identical to a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Every element is driven, and the assignment stays live: a later change to
/// the source propagates.
#[test]
fn whole_array_assign_drives_every_element_and_tracks_updates() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic [9:0] src [4];
  logic [9:0] dst [4];
  assign dst = src;
  int d0, d1, d2, d3, d1_after;
  initial begin
    src[0] = 10'h011; src[1] = 10'h022; src[2] = 10'h033; src[3] = 10'h044;
    #1;
    d0 = dst[0]; d1 = dst[1]; d2 = dst[2]; d3 = dst[3];
    src[1] = 10'h077;
    #1 d1_after = dst[1];
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "d0"), 0x011);
    assert_eq!(u(&sim, "d1"), 0x022);
    assert_eq!(u(&sim, "d2"), 0x033);
    assert_eq!(u(&sim, "d3"), 0x044);
    assert_eq!(u(&sim, "d1_after"), 0x077, "the assign stays live after the source changes");
}

/// An array whose element type is a packed struct — the shape the CDC design
/// used — and index ranges that do not start at 0.
#[test]
fn whole_array_assign_of_struct_elements_and_offset_ranges() {
    let src = r#"
`timescale 1ns/1ns
module top;
  typedef struct packed { logic [9:0] f; } hf_t;
  hf_t hsrc [4];
  hf_t hdst [4];
  logic [7:0] osrc [1:4];
  logic [7:0] odst [1:4];
  assign hdst = hsrc;
  assign odst = osrc;
  int h0, h3, o1, o4;
  initial begin
    hsrc[0] = 10'h055; hsrc[3] = 10'h0AA;
    osrc[1] = 8'h5A;   osrc[4] = 8'hC3;
    #1;
    h0 = hdst[0]; h3 = hdst[3]; o1 = odst[1]; o4 = odst[4];
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "h0"), 0x055, "struct-element array, first");
    assert_eq!(u(&sim, "h3"), 0x0AA, "struct-element array, last");
    assert_eq!(u(&sim, "o1"), 0x5A, "1-based range, first");
    assert_eq!(u(&sim, "o4"), 0xC3, "1-based range, last");
}

/// The guard: forms that already worked must be untouched — per-element
/// assigns, a packed 2D whole assign, and a plain scalar assign.
#[test]
fn existing_assign_forms_are_unchanged() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic [9:0] src [4];
  logic [9:0] per [4];
  logic [3:0][9:0] psrc, pdst;
  logic [7:0] a, b;
  assign per[0] = src[0];
  assign per[1] = src[1];
  assign pdst = psrc;
  assign b = a;
  int p0, p1, pk, sc;
  initial begin
    src[0] = 10'h011; src[1] = 10'h022;
    psrc = {10'h004, 10'h003, 10'h002, 10'h001};
    a = 8'h5A;
    #1;
    p0 = per[0]; p1 = per[1]; pk = pdst[2]; sc = b;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "p0"), 0x011, "per-element assign");
    assert_eq!(u(&sim, "p1"), 0x022);
    assert_eq!(u(&sim, "pk"), 0x003, "packed 2D whole assign");
    assert_eq!(u(&sim, "sc"), 0x5A, "plain scalar assign");
}
