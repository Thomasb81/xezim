//! An UNPACKED ARRAY member of an unpacked struct (`struct { logic [15:0] a [4]; }`).
//!
//! Its elements are individual signals (`s.a[0]` …) and the base `s.a` is not
//! a signal at all, so `s.a[i] = v` matched no lvalue arm and degraded into a
//! bit-select of a phantom scalar — the write was discarded and the member read
//! back x forever. Reads had the mirror-image problem.
//!
//! That is quiet in the worst way: a scoreboard whose expected-value struct has
//! an array member compares x against every DUT output, so it reports a
//! mismatch on every lane of every cycle while the DUT is perfectly correct.
//! Found from exactly such a testbench.
//!
//! A module-level struct has its element signals pre-registered; a PROCEDURAL
//! LOCAL one has nothing registered, so the write creates the element (only for
//! a member path — a plain local vector's bit-write is never diverted).
//!
//! KNOWN GAP, still open: pushing such a struct through a QUEUE loses the
//! members (`scratchpad/tb/e1.sv` E1D, `e3.sv` E3C), as does a hierarchical
//! `dut.result.member[i]` select on a packed-2D struct member (`e4.sv` E4B).
//!
//! Verified byte-identical to a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Write then read back an array member of a module-level struct, and copy the
/// whole struct.
#[test]
fn module_level_struct_array_member_round_trips() {
    let src = r#"
module top;
  typedef struct { logic [15:0] arr [4]; int scalar; } s_t;
  s_t a, b;
  int r0, r3, sc, c0, c3, csc;
  initial begin
    a.arr[0] = 16'h1111; a.arr[3] = 16'h4444; a.scalar = 42;
    r0 = a.arr[0]; r3 = a.arr[3]; sc = a.scalar;
    b = a;
    c0 = b.arr[0]; c3 = b.arr[3]; csc = b.scalar;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r0"), 0x1111, "element 0 reads back what was written");
    assert_eq!(u(&sim, "r3"), 0x4444, "element 3 likewise");
    assert_eq!(u(&sim, "sc"), 42, "the scalar member still works");
    assert_eq!(u(&sim, "c0"), 0x1111, "struct copy carries the array member");
    assert_eq!(u(&sim, "c3"), 0x4444);
    assert_eq!(u(&sim, "csc"), 42, "and the scalar");
}

/// A PROCEDURAL-LOCAL struct: nothing is pre-registered, so the element is
/// created on first write.
#[test]
fn procedural_local_struct_array_member_round_trips() {
    let src = r#"
module top;
  typedef struct { logic [15:0] arr [4]; int scalar; } s_t;
  s_t mod_level;
  int l0, l3, m0, msc;
  initial begin
    s_t loc;
    loc.arr[0] = 16'h1111; loc.arr[3] = 16'h4444; loc.scalar = 7;
    l0 = loc.arr[0]; l3 = loc.arr[3];
    mod_level = loc;
    m0 = mod_level.arr[0]; msc = mod_level.scalar;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "l0"), 0x1111, "a local struct's array member stores");
    assert_eq!(u(&sim, "l3"), 0x4444);
    assert_eq!(u(&sim, "m0"), 0x1111, "and copies out to a module-level struct");
    assert_eq!(u(&sim, "msc"), 7);
}

/// A variable index, and each element independent of its siblings.
#[test]
fn array_member_elements_are_independent_under_a_variable_index() {
    let src = r#"
module top;
  typedef struct { logic [15:0] arr [4]; } s_t;
  s_t s;
  int e0, e1, e2, e3;
  initial begin
    for (int i = 0; i < 4; i++) s.arr[i] = 16'h1000 + i;
    e0 = s.arr[0]; e1 = s.arr[1]; e2 = s.arr[2]; e3 = s.arr[3];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "e0"), 0x1000);
    assert_eq!(u(&sim, "e1"), 0x1001);
    assert_eq!(u(&sim, "e2"), 0x1002);
    assert_eq!(u(&sim, "e3"), 0x1003);
}

/// The guard: an ordinary packed-vector bit-write must NOT be diverted into the
/// element path — `v` is a signal of its own, the discriminator the fix uses.
#[test]
fn packed_vector_bit_writes_are_unaffected() {
    let src = r#"
module top;
  logic [7:0] v;
  logic [3:0][7:0] p2;
  int rv, rp;
  initial begin
    v = 8'h00;
    v[3] = 1'b1;
    v[0] = 1'b1;
    rv = v;
    p2 = '0;
    p2[2] = 8'hAB;
    rp = p2[2];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "rv"), 0x09, "bit-writes still target bits");
    assert_eq!(u(&sim, "rp"), 0xAB, "packed 2D element write unaffected");
}
