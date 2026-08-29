//! §8.9 — a `static` collection (queue / assoc array) is one copy PER CLASS,
//! shared across instances/subclasses but NOT across distinct classes. Two
//! sibling classes declaring a same-named `static … settings[$]` must get
//! separate stores. xezim materialized every static collection under its bare
//! name, so the two siblings shared one cell: a module-scope read of either
//! (`A.settings[k]`, `.size()`) or an in-method bare access collided. In UVM
//! this made the four sibling `uvm_cmdline_*` classes' `settings` queues share
//! storage, so a verbosity `+UVM_VERBOSITY=` setting leaked into the
//! `uvm_set_action=/uvm_set_severity=/uvm_set_verbosity=` checkers, emitting
//! spurious "never took effect" INVLCMDARGS warnings that inflated the warning
//! severity count and broke self-checking report tests. Verified byte-for-byte
//! against a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal {} not found", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}
/// Two sibling classes declaring a same-named static queue must keep
/// independent stores; writes and index reads must not bleed across classes.
#[test]
fn sibling_classes_same_named_static_queue_independent() {
    let src = r#"
module top;
  class base; endclass
  class A extends base;
    static int settings[$];
  endclass
  class B extends base;
    static int settings[$];
  endclass
  int ra, rb, a0, b0;
  initial begin
    A::settings.push_back(1);
    B::settings.push_back(2);
    ra = A::settings.size();
    rb = B::settings.size();
    a0 = A::settings[0];
    b0 = B::settings[0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "ra"), 1, "A.settings holds its own single element");
    assert_eq!(u(&sim, "rb"), 1, "B.settings holds its own single element");
    assert_eq!(u(&sim, "a0"), 1, "A.settings[0] is A's write");
    assert_eq!(u(&sim, "b0"), 2, "B.settings[0] is B's write");
}

/// Control: a static collection declared by exactly ONE class keeps its
/// bare-name store (sibling separation must not split single-class storage).
#[test]
fn single_declaring_class_static_queue_unchanged() {
    let src = r#"
module top;
  class S;
    static int log[$];
  endclass
  int n, first;
  initial begin
    S::log.push_back(9);
    n = S::log.size();
    first = S::log[0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "n"), 1);
    assert_eq!(u(&sim, "first"), 9);
}

/// A same-named static collection shared through an INHERITED position still
/// resolves to the DECLARING class's single store (subclasses share the base
/// static — §8.9), across both A and B accesses.
#[test]
fn inherited_static_queue_shared_through_subclasses() {
    let src = r#"
module top;
  class S;
    static int cache[$];
  endclass
  class A extends S; endclass
  class B extends S; endclass
  int n, first;
  initial begin
    A::cache.push_back(3);
    B::cache.push_back(4);
    n = A::cache.size();   // same store as B's push
    first = A::cache[0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // `cache` is declared in S alone (no sibling collision), so bare-name
    // storage is shared by both subclasses.
    assert_eq!(u(&sim, "n"), 2);
    assert_eq!(u(&sim, "first"), 3);
}