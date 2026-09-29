//! class-perf tiering: with the gate on, a method body compiles only after
//! `XEZIM_METHOD_TIER` calls (the product default is a much larger,
//! economics-calibrated value — see `method_tier_threshold` in the
//! simulator; the boundary arm below pins its own threshold so its
//! coverage never depends on that default). Below the threshold every call
//! runs the AST interpreter with gate-OFF frames; above it the compiled
//! block takes over. The ON==OFF byte-for-byte contract must therefore hold
//! ACROSS the tier boundary: a hot method must observe its own state
//! (static + instance) identically whether a given call was interpreted or
//! compiled, including values computed before the boundary and consumed
//! after it.
//!
//! The test drives one method past the default threshold (250 outer / 750
//! inner calls vs 100) and checks the accumulated results against an
//! independent Rust model, then re-runs the same source with tier 0
//! (compile-on-first-call) and a huge tier (never compiles) — all three
//! must agree, proving the boundary itself is invisible.

use xezim::simulate;

fn gate_on() {
    // Safety: tests run in one process; set/reset is sequential here.
    unsafe { std::env::set_var("XEZIM_COMPILE_METHODS", "1") };
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
class counter;
  static longint unsigned s_hits;
  int acc;
  function new();
    acc = 1000;
  endfunction
  function int step(int v);
    acc = acc + v;
    s_hits = s_hits + 1;
    if (acc > 1100) acc = acc - 7;
    return acc;
  endfunction
  function int sum3(int a, int b, int c);
    return step(a) + step(b) + step(c);
  endfunction
endclass

module top;
  counter c;
  longint unsigned total;
  longint unsigned hits;
  int racc;
  int r;
  int i;
  initial begin
    c = new();
    for (i = 0; i < 250; i = i + 1) begin
      r = c.sum3(i, i + 1, 2);
      total = total + r;
    end
    hits = counter::s_hits;
    racc = c.acc;
  end
endmodule
"#;

/// Independent model: the exact `step`/`sum3` semantics, no SV involved.
fn model() -> (i64, i64, i64) {
    let (mut acc, mut s_hits, mut total) = (1000i64, 0i64, 0i64);
    for i in 0..250i64 {
        let mut sum = 0i64;
        for v in [i, i + 1, 2] {
            acc += v;
            s_hits += 1;
            if acc > 1100 {
                acc -= 7;
            }
            sum += acc;
        }
        total += sum;
    }
    (total, s_hits, acc)
}

fn run() -> (u64, u64, u64) {
    let sim = simulate(SRC, 100).expect("simulation should run");
    (u(&sim, "total"), u(&sim, "hits"), u(&sim, "racc"))
}

#[test]
fn tiering_boundary_is_invisible() {
    let (want_total, want_hits, want_acc) = model();
    // Sanity: the model actually exercises the >1100 clamp.
    assert!(want_hits == 750, "model must make 750 step() calls");

    // Pinned tier 100 (not the product default): the first 100 step()
    // calls interpret, the remaining 650 run compiled — the boundary
    // fires mid-run.
    gate_on();
    unsafe { std::env::set_var("XEZIM_METHOD_TIER", "100") };
    let a = run();
    assert_eq!(a, (want_total as u64, want_hits as u64, want_acc as u64));

    // Tier 0: compiled from the very first call.
    unsafe { std::env::set_var("XEZIM_METHOD_TIER", "0") };
    let b = run();
    assert_eq!(b, a, "tier 0 (all compiled) diverged from default tier");

    // Huge tier: every call interpreted, gate effectively cold.
    unsafe { std::env::set_var("XEZIM_METHOD_TIER", "1000000") };
    let c = run();
    assert_eq!(c, a, "huge tier (all interpreted) diverged");

    // Reset so later tests in this process see the default policy.
    unsafe {
        std::env::remove_var("XEZIM_METHOD_TIER");
        std::env::remove_var("XEZIM_COMPILE_METHODS");
    }
}
