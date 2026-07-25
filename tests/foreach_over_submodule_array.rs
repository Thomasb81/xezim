//! §12.7.3 — a `foreach` over an array declared inside an instantiated
//! SUBMODULE resolved the array name unscoped.
//!
//! The existing scoping used `name_resolve_hint`, which is only installed while
//! a PROCESS runs. Two things went wrong from there:
//!
//!   * the loop BOUNDS: `foreach_dims` looked up the bare name, missed, and
//!     returned None, so a multi-var `foreach (m[i,j])` never iterated its
//!     rectangle;
//!   * the body WRITE: `m[i][j] = v` resolved the bare base name, found no
//!     registered array, and the write was silently dropped.
//!
//! Both now fall back to the scope hint and then to a UNIQUE suffix match —
//! `<scope>.m` is accepted only when exactly one registered array ends that
//! way, so this can never choose between same-named arrays in sibling
//! instances.
//!
//! STILL OPEN (deliberately not asserted): the same `foreach` inside an
//! `always_comb`, and the 1-D element-write path, both still drop their writes
//! in a submodule. They are evaluated from the settle path, where the per-node
//! resolved-name cache can already hold the unscoped form. Reference-verified
//! as wrong; tracked separately.

use xezim::simulate;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

const SRC: &str = r#"
module holder (output [15:0] res_a, res_b);
  logic [15:0] grid [0:1][0:1];
  initial foreach (grid[i, j]) grid[i][j] = 16'hBEEF;
  assign res_a = grid[0][0];
  assign res_b = grid[1][1];   // last element: proves the rectangle iterated
endmodule

module tb;
  wire  [15:0] res_a, res_b;
  logic [15:0] seen_a, seen_b;
  holder dut (.res_a(res_a), .res_b(res_b));
  initial begin
    #3;
    seen_a = res_a;
    seen_b = res_b;
  end
endmodule
"#;

#[test]
fn foreach_over_a_two_dim_submodule_array_writes_every_element() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(get(&sim, "seen_a") & 0xFFFF, 0xBEEF);
    assert_eq!(get(&sim, "seen_b") & 0xFFFF, 0xBEEF);
}
