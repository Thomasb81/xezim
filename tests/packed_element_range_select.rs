//! §7.4.1 — a constant RANGE select on a packed multi-dimensional base selects
//! ELEMENTS, not bits. `pv[1:0]` on `logic [1:0][63:0] pv` is both 64-bit
//! slices (128 bits); on a packed array of a 128-bit struct typedef it is 256
//! bits. Three places treated it as a plain bit range:
//!
//!   * the simulator's `RangeSelect` eval (single-ELEMENT select was already
//!     element-aware; the range form was not),
//!   * the bytecode read-side compile (`RangeSelectConst` with unscaled
//!     bounds),
//!   * the port-connection width computation, which then reported
//!     "port is 256 bit(s) but the connection is 2 bit(s)" and mis-sized the
//!     port continuous assign — a customer port actual written as
//!     `.p(arr[1:0])` carried 2 bits into a 256-bit port and the data was
//!     lost.
//!
//! Bounds are normalized against the DECLARED outer dimension, so ascending
//! ranges select the right slots. A 1-D vector keeps the plain bit-range
//! meaning. All expectations reference-simulator verified, including the
//! ascending and partial-range cases.

use xezim::simulate;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

const SRC: &str = r#"
package lanes_pkg;
  typedef struct packed { logic [1:0][31:0] words; } bundle_t;   // 64 bits
endpackage
import lanes_pkg::*;

module taker (input bundle_t [1:0] pair_in, output logic [127:0] flat_out);
  assign flat_out = pair_in;
endmodule

module tb;
  logic [3:0][7:0] pv;        // descending outer
  logic [0:3][7:0] pa;        // ascending outer
  logic [63:0]     plain;     // 1-D control: [k:j] stays a bit range
  bundle_t [1:0]   arr;       // packed array of struct typedef, 128 bits
  wire [127:0]     flat;

  taker u_taker (.pair_in(arr[1:0]), .flat_out(flat));

  int w_desc_rng, w_desc_one, w_asc_rng, w_plain_rng, w_tdef_rng;
  logic [15:0] v_desc, v_asc;
  logic [7:0]  v_plain;
  logic [63:0] v_low, v_high;

  initial begin
    pv    = 32'h44_33_22_11;
    pa    = 32'hAA_BB_CC_DD;
    plain = 64'hFEDC_BA98_7654_3210;
    arr[0] = 64'h1111_2222_3333_4444;
    arr[1] = 64'hAAAA_BBBB_CCCC_DDDD;
    #1;
    w_desc_rng  = $bits(pv[2:1]);     // 2 elements x 8
    w_desc_one  = $bits(pv[1:1]);     // 1 element
    w_asc_rng   = $bits(pa[1:2]);     // 2 elements x 8
    w_plain_rng = $bits(plain[7:0]);  // plain vector: 8 BITS
    w_tdef_rng  = $bits(arr[1:0]);    // 2 structs x 64
    v_desc  = pv[2:1];
    v_asc   = pa[1:2];
    v_plain = plain[7:0];
    v_low   = flat[63:0];
    v_high  = flat[127:64];
  end
endmodule
"#;

#[test]
fn constant_range_on_packed_multi_dim_selects_elements() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Widths.
    assert_eq!(get(&sim, "w_desc_rng"), 16);
    assert_eq!(get(&sim, "w_desc_one"), 8);
    assert_eq!(get(&sim, "w_asc_rng"), 16);
    assert_eq!(get(&sim, "w_plain_rng"), 8); // unchanged bit-range meaning
    assert_eq!(get(&sim, "w_tdef_rng"), 128);
    // Values, including slot order for the ascending declaration.
    assert_eq!(get(&sim, "v_desc") & 0xFFFF, 0x3322);
    assert_eq!(get(&sim, "v_asc") & 0xFFFF, 0xBBCC);
    assert_eq!(get(&sim, "v_plain") & 0xFF, 0x10);
    // The whole 128 bits crossed the port.
    assert_eq!(get(&sim, "v_low"), 0x1111_2222_3333_4444);
    assert_eq!(get(&sim, "v_high"), 0xAAAA_BBBB_CCCC_DDDD);
}
