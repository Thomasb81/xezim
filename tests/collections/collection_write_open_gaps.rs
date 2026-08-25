//! Collection / class write gaps found by audit rounds 8 and 9 (2026-08-24).
//! Both were PRE-EXISTING (the Aug-22 baseline behaves identically). Round 9 is
//! now FIXED and runs live; round 8 is still open and stays `#[ignore]`d rather
//! than deleted, so it starts passing loudly when fixed.

use xezim::simulate;

/// ROUND 8 — a CONTINUOUS ASSIGN reading a dynamic-array element.
///
/// With an `assign x = d[i];` present, an NBA write to that array never
/// reaches ANY reader, and the readers stay `x` permanently — a later
/// blocking write to the same array does not recover them. Without the
/// continuous assign the very same NBA write is correct, so the CA's presence
/// is what changes the binding. Blocking writes are correct either way, and a
/// static array with a CA reader is correct, which is what scopes this.
///
/// The NBA does take the intended path — `resolve_nba_target` returns None for
/// dynamic arrays and it commits through `assign_value`, verified with a
/// temporary probe — so the divergence is downstream, in what re-settles a
/// continuous assign that reads a collection.
const CA_READER_ON_DYNAMIC: &str = r#"
module tb;
  logic [7:0] c [];
  logic [7:0] via_ca, via_comb;
  int ok;
  assign      via_ca   = c[0];
  always_comb via_comb = c[0];
  initial begin
    c = new[4];
    for (int k = 0; k < 4; ++k) c[k] <= 8'd33 + k[7:0];
    #1;
    ok = (via_ca == 8'd33) && (via_comb == 8'd33);
  end
endmodule
"#;

/// ROUND 9 — a NON-BLOCKING write to a CLASS PROPERTY (audit round 9; FIXED).
///
/// Far broader than the collection bugs: EVERY `prop <= val` inside a class
/// method was silently dropped — scalars and vectors as much as static arrays,
/// queues and dynamic arrays — while a blocking write in the same method
/// worked, so the value simply never arrived.
///
/// Cause: the NBA slow path commits through `assign_value` in the NBA region,
/// AFTER the process context is restored to the event-loop caller, so
/// `this_stack` was empty there and the class-property target resolved to
/// nothing. Fixed by capturing the `this` handle at SCHEDULE time in
/// `NbaEntry::this_handle` and re-pushing it for the apply — the same shape as
/// the existing `static_key`, which pins the declaring frame for `static`
/// subroutine locals.
const NBA_TO_CLASS_MEMBER_ARRAY: &str = r#"
class Bag;
  int         sc;
  logic [7:0] v;
  logic [7:0] d [];
  logic [7:0] s [3:0];
  logic [7:0] q [$];
  function new();
    d = new[4];
    q.delete();
    for (int k = 0; k < 4; ++k) q.push_back(8'd0);
  endfunction
  function void nba_all(int i);
    sc <= 7;
    v  <= 8'd9;
    for (int k = 0; k < 4; ++k) d[k] <= 8'd20 + k[7:0];
    for (int k = 0; k < 4; ++k) s[k] <= 8'd30 + k[7:0];
    for (int k = 0; k < 4; ++k) q[k] <= 8'd40 + k[7:0];
    d[i] <= 8'd21;                      // variable index
    s[i] <= 8'd31;
  endfunction
endclass
module tb;
  Bag b;
  int ok;
  initial begin
    b = new();
    b.nba_all(1);
    #1;
    ok = (b.sc == 7) && (b.v == 8'd9)
      && (b.d[0] == 8'd20) && (b.s[0] == 8'd30) && (b.q[0] == 8'd40)
      && (b.d[1] == 8'd21) && (b.s[1] == 8'd31);
  end
endmodule
"#;

fn ok_flag(src: &str) -> u64 {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .expect("signal 'ok' not found")
        .to_u64()
        .unwrap_or(0)
}

#[test]
#[ignore = "open gap: a continuous-assign reader of a dynamic array never sees NBA writes (audit round 8)"]
fn continuous_assign_reader_of_dynamic_array_sees_nba_writes() {
    assert_eq!(ok_flag(CA_READER_ON_DYNAMIC), 1);
}

#[test]
fn nba_to_class_property_is_applied() {
    assert_eq!(ok_flag(NBA_TO_CLASS_MEMBER_ARRAY), 1);
}
