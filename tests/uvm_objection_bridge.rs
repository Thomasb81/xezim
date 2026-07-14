//! Genuine-UVM-library objection `wait_for` bridging regression.
//!
//! The genuine UVM scheduler ends `uvm_phase_hopper::run_phases` with
//! `wait_for_objection(UVM_ALL_DROPPED)`, which routes its enum PARAMETER
//! through to the inner `objection.wait_for(objt_event, obj)`. xezim bridges
//! that call — the real body blocks on `@(m_events[obj].all_dropped)`, an
//! event member of an assoc-array-indexed object xezim can't key, so the `@`
//! resolves to an empty sensitivity and the phase loop spins at t0.
//!
//! Two bugs previously defeated the bridge:
//!  1. It matched only a LITERAL `UVM_ALL_DROPPED` ident. The routed
//!     VARIABLE-arg call (`objt_event`) fell through to the empty `@()`.
//!  2. A single `wait(total == 0)` returned immediately when the total was 0
//!     at entry — *before* the run phase had raised — ending the schedule
//!     at t0.
//!
//! The fix resolves the event arg by EVALUATION (literal OR variable) and
//! uses the raise-then-drop idiom `wait(total > 0); wait(total == 0)` for
//! UVM_ALL_DROPPED. This test exercises both facets directly with a
//! self-contained objection-like class (no UVM library dependency), so it
//! is fast and deterministic.

use xezim::simulate;

const SRC: &str = r#"
class objection;
  int total;
  function new; total = 0; endfunction
  function void raise_objection(input objection o); total = total + 1; endfunction
  function void drop_objection(input objection o);  total = total - 1; endfunction
  function int  get_objection_total(input objection o); return total; endfunction
  // The genuine uvm_objection::wait_for body blocks on an event member xezim
  // cannot key. If the bridge fails to rewrite this call, execution reaches
  // `@(total)` (empty sensitivity, since `total` is a class field) and the
  // waiter never returns.
  task wait_for(int evt, objection o);
    @(total);
  endtask
endclass

module top;
  initial begin
    objection o;
    o = new;
    // raiser: raise at t=50, drop at t=60.
    fork
      begin
        #50; o.raise_objection(o);
        #10; o.drop_objection(o);
        $display("RAISER_DONE at %0t", $time);
      end
    join_none
    // VARIABLE-arg wait_for — the routed-parameter case that the literal-only
    // matcher used to miss. evt = 4 = UVM_ALL_DROPPED.
    begin
      int evt;
      evt = 4;
      o.wait_for(evt, o);
      $display("WAITER_DONE at %0t", $time);
    end
  end
endmodule
"#;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn wait_for_with_variable_arg_bridges_to_raise_then_drop() {
    // The bridge is gated on the genuine-UVM path (PURE_SV_LRM=1, the default
    // since 7fc8187). Set it explicitly so the test is deterministic.
    std::env::set_var("PURE_SV_LRM", "1");
    let sim = simulate(SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);

    let waiter = msgs
        .iter()
        .find(|m| m.starts_with("WAITER_DONE"))
        .unwrap_or_else(|| {
            panic!(
                "wait_for never returned (bridge did not fire / did not block); \
                 output: {:?}",
                msgs
            )
        });
    // raise at t=50, drop at t=60 → the raise-then-drop idiom releases the
    // waiter at t=60. A bare `wait(total==0)` would release at t=0; a
    // non-bridged call would never release (hang on @(total)).
    assert!(
        waiter.contains("at 60"),
        "expected waiter released at t=60 after raise+drop, got: {}",
        waiter
    );
}
