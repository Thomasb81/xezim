//! ivtest round 4 — four defects, all reference-validated.
//!
//! 1. **§6.19 enums declared inside a struct member's type** install their
//!    members in the scope enclosing the struct (`struct packed { enum
//!    integer { A } e; } s;` makes `A` visible) — the registration never
//!    recursed into struct members (`enum_in_struct`).
//! 2. **§6.19 X/Z-valued enum members** (`XX = 'bx`, `XZ = 32'h1x2z3xxz`):
//!    the u64 member pipeline masked the x bits to 0. The registered constant
//!    is now rebuilt as a 4-state Value when the initializer carries x/z
//!    (`enum_test1`).
//! 3. **Package variables with packed multi-D types** (`reg [1:0][7:0] y`)
//!    lacked the packed-shape maps, so `P::y[0]` read 0
//!    (`package_vec_part_select`).
//! 4. **Ports declared with an ARRAY typedef** (`typedef logic [A-1:0] T[B];
//!    input T x;`) inherit the typedef's unpacked dims — the ANSI-port path
//!    registered a scalar of the element width, so `$size(x,1)` reported the
//!    packed width (`module_port_typedef_array1`). Root cause was twofold:
//!    the port arm never consulted `typedef_unpacked_dims`, and the
//!    capture-time dim fold skipped `[B]` because a parameter-named dim
//!    parses as an ASSOCIATIVE dimension keyed by "type B".

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("test.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Enum inside a struct member type: members visible in the enclosing scope.
#[test]
fn enum_inside_struct_member_installs_members() {
    let src = r#"
module test;
  struct packed {
    enum integer { SA = 3, SB } e;
  } s;
  int va, vb, eq;
  initial begin
    s.e = SA;
    va = SA; vb = SB;
    eq = (s.e == SA);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "va"), 3);
    assert_eq!(u(&sim, "vb"), 4, "auto-increment continues");
    assert_eq!(u(&sim, "eq"), 1);
}

/// X/Z-valued enum members keep their 4-state pattern.
#[test]
fn enum_members_with_xz_values() {
    let src = r#"
module test;
  enum integer { IDLE, XX = 'bx, XY = 'b01, YY = 'b10, XZ = 32'h1x2z3xxz } ns;
  int xx_is_x, xz_exact, xy_v;
  initial begin
    xx_is_x  = (XX === 'bx);
    xz_exact = (XZ === 32'h1x2z3xxz);
    xy_v     = XY;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "xx_is_x"), 1, "XX === 'bx");
    assert_eq!(u(&sim, "xz_exact"), 1, "the full x/z pattern survives");
    assert_eq!(u(&sim, "xy_v"), 1, "plain members unchanged");
}

/// Package vars: part-selects and packed-2D element selects through P::.
#[test]
fn package_var_selects() {
    let src = r#"
package P;
  reg [7:0] x = 8'h5a;
  reg [1:0][7:0] y = 16'h5af0;
endpackage
module test;
  int lo, hi, y0, y1;
  initial begin
    lo = P::x[3:0]; hi = P::x[7:4];
    y0 = P::y[0]; y1 = P::y[1];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "lo"), 0xA);
    assert_eq!(u(&sim, "hi"), 0x5);
    assert_eq!(u(&sim, "y0"), 0xF0, "packed-2D element through P::");
    assert_eq!(u(&sim, "y1"), 0x5A);
}

/// A port declared with an array typedef gets the typedef's unpacked shape.
#[test]
fn typedef_array_port_inherits_unpacked_dims() {
    let src = r#"
localparam A = 2;
localparam B = 4;
typedef logic [A-1:0] T[B];
module test (input T x);
  int s1, s2, b, d;
  initial begin
    s1 = $size(x, 1);
    s2 = $size(x, 2);
    b  = $bits(x);
    d  = $dimensions(x);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "s1"), 4, "dim 1 is the unpacked [B]");
    assert_eq!(u(&sim, "s2"), 2, "dim 2 is the packed [A-1:0]");
    assert_eq!(u(&sim, "b"), 8, "4 elements x 2 bits");
    assert_eq!(u(&sim, "d"), 2);
}

/// §7.4.2: packed dims AFTER an enum body (`enum {...} [1:0] x;`) make a
/// packed array of the enum — mirroring the struct body-suffix form
/// (ivtest `array_packed`).
#[test]
fn enum_body_suffix_packed_dims() {
    let src = r#"
module test;
  typedef enum logic [7:0] { A } E;
  E [1:0] ep2;
  enum logic [7:0] { B } [1:0] ep3;
  int b2, b3;
  initial begin
    b2 = $bits(ep2);
    b3 = $bits(ep3);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "b2"), 16, "typedef'd enum with packed dims");
    assert_eq!(u(&sim, "b3"), 16, "anonymous enum with body-suffix dims");
}

/// §7.2.1: a packed-struct member read carries the member's DECLARED
/// signedness — the slice itself is a raw bit pattern (ivtest
/// `struct_packed_sysfunct2`: `%0d` of an `int` member printed unsigned).
#[test]
fn struct_member_reads_keep_declared_signedness() {
    let src = r#"
module test;
  struct packed { int s; int unsigned u; } x;
  int neg, via_int, both_u;
  initial begin
    x.s = -20;
    x.u = -10;
    neg     = (x.s < 0);
    via_int = x.s;
    both_u  = (x.u > 0);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "neg"), 1, "int member compares signed");
    assert_eq!(u(&sim, "via_int") as u32 as i32, -20);
    assert_eq!(u(&sim, "both_u"), 1, "unsigned member stays unsigned");
}
