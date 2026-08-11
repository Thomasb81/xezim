//! Post-closure fresh-audit finds — reference-validated.
//!
//! 1. §7.12: a DECLARED iterator (`q.sort(x) with (x)`) was silently
//!    ignored — the filter evaluated 0 for every element, so sort became a
//!    stable no-op and `with`-reductions summed zeros. The iterator name now
//!    binds alongside `item` in sort/rsort/unique and the reductions.
//! 2. §9.4.2/§7.2.1: `@(s.a)` on a PACKED STRUCT field armed a nonexistent
//!    signal (the field is a slice of the base vector) and woke spuriously
//!    at t=0. It now arms the BASE with the field expression as the
//!    value-compare term.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Reference: sorted '{1,2,4,5}; after delete(1): '{1,4,5} sum=10.
#[test]
fn sort_with_declared_iterator() {
    let src = r#"
module tb;
  int q[$] = '{5, 1, 4, 2};
  initial begin
    q.sort(x) with (x);
    $display("T|sorted=%p", q);
    q.delete(1);
    $display("T|afterdel=%p sum=%0d", q, q.sum());
    $display("T|wsum=%0d", q.sum(y) with (y * 2));
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let o = outs(&sim);
    assert!(o.contains(&"T|sorted='{1, 2, 4, 5}".to_string()), "{o:?}");
    assert!(o.contains(&"T|afterdel='{1, 4, 5} sum=10".to_string()), "{o:?}");
    assert!(o.contains(&"T|wsum=20".to_string()), "named iterator in reductions: {o:?}");
}

/// Reference: seen=4 — the wait parks until the FIELD changes.
#[test]
fn event_control_on_packed_struct_field() {
    let src = r#"
module tb;
  typedef struct packed { logic a; logic b; } sp_t;
  sp_t s = '0;
  int seen = -1;
  initial begin
    fork
      begin @(s.a); seen = $time; end
      begin #4 s.a = 1; end
    join
    $display("T|seen=%0d", seen);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(outs(&sim).contains(&"T|seen=4".to_string()), "{:?}", outs(&sim));
}
