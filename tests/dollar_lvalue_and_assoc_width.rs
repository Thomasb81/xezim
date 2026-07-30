//! Two gaps found by auditing the batch 14-16 fixes. Both predate that work —
//! each was proven pre-existing before being fixed — and both are now
//! reference-validated.
//!
//! 1. §11.4.12 — `$` in a QUEUE index is the queue's last valid index. The READ
//!    paths push `dollar_bound` before evaluating the index; no LVALUE path
//!    did, so `ExprKind::Dollar` fell through to its `u64::MAX` default and
//!    `q[$] = v` wrote element 18446744073709551615: the write silently
//!    vanished, for blocking and non-blocking alike. The index is now
//!    normalised once, with the bound installed, at the top of the indexed-
//!    lvalue arm — plus in `resolve_nba_target` and `freeze_lvalue_indices`,
//!    which each evaluate it on their own path.
//!
//! 2. §10.7 — an associative-array element has no entry in the typed signal
//!    table, and the elaborator recorded only whether the array was
//!    string-keyed, never its ELEMENT WIDTH. So a write stored the RHS at its
//!    own size: `logic [3:0] aa[string]; aa["k"] = 8'hEF` kept all eight bits
//!    and `$bits` reported 8. `ElaboratedModule::assoc_elem_widths` now carries
//!    the declared width and the store fits to it.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// `q[$]` as an lvalue, blocking and non-blocking, against a constant-index
/// control that always worked.
#[test]
fn dollar_index_lvalue_targets_the_last_element() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int qa [$]; int qb [$]; int qc [$];
  int r_blk, r_nba, r_const, sz;
  initial begin
    qa.push_back(10); qa.push_back(20);
    qb.push_back(10); qb.push_back(20);
    qc.push_back(10); qc.push_back(20);
    @(posedge clk);
    qa[$] = 77;       // blocking
    qb[$] <= 88;      // non-blocking
    qc[1] <= 99;      // control: constant index
    @(posedge clk); #1;
    r_blk = qa[1]; r_nba = qb[1]; r_const = qc[1]; sz = qa.size();
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r_blk"), 77, "blocking q[$] must hit the last element");
    assert_eq!(u(&sim, "r_nba"), 88, "non-blocking q[$] must too");
    assert_eq!(u(&sim, "r_const"), 99, "constant index unaffected");
    assert_eq!(u(&sim, "sz"), 2, "writing q[$] must not grow the queue");
}

/// A `$`-relative expression (`q[$-1]`) resolves against the same bound.
#[test]
fn dollar_relative_index_lvalue() {
    let src = r#"
`timescale 1ns/1ns
module top;
  int q [$];
  int first, last;
  initial begin
    q.push_back(10); q.push_back(20); q.push_back(30);
    q[$-2] = 55;      // first element
    q[$]   = 66;      // last element
    #1 first = q[0]; last = q[2];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "first"), 55, "q[$-2] is the first of three");
    assert_eq!(u(&sim, "last"), 66, "q[$] is the last");
}

/// Assoc elements take their DECLARED width — narrow truncates, wide extends —
/// and blocking and non-blocking agree.
#[test]
fn associative_element_writes_fit_the_declared_width() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] narrow_blk [string];
  logic [3:0] narrow_nba [string];
  int         wide_blk   [string];
  int         wide_nba   [string];
  int nb, nn, wb, wn, bits_n;
  initial begin
    @(posedge clk);
    narrow_blk["k"]  = 8'hEF;
    narrow_nba["k"] <= 8'hEF;
    wide_blk["k"]    = 8'hEF;
    wide_nba["k"]   <= 8'hEF;
    @(posedge clk); #1;
    nb = narrow_blk["k"]; nn = narrow_nba["k"];
    wb = wide_blk["k"];   wn = wide_nba["k"];
    bits_n = $bits(narrow_blk["k"]);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "nb"), 0xF, "4-bit element truncates 8'hEF");
    assert_eq!(u(&sim, "nn"), 0xF, "and the NBA path agrees");
    assert_eq!(u(&sim, "wb"), 0xEF, "an int element keeps the value");
    assert_eq!(u(&sim, "wn"), 0xEF, "NBA likewise");
    assert_eq!(u(&sim, "bits_n"), 4, "$bits reports the DECLARED width");
}

/// Queues and dynamic arrays keep fitting to their element width — the assoc
/// change must not have moved them onto the untyped path.
#[test]
fn queue_and_dynamic_elements_still_fit() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] q [$];
  logic [3:0] dyn [];
  int rq, rd;
  initial begin
    q.push_back(4'h0); q.push_back(4'h0);
    dyn = new[2];
    @(posedge clk);
    q[1]   <= 8'hAB;
    dyn[1] <= 8'hCD;
    @(posedge clk); #1;
    rq = q[1]; rd = dyn[1];
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "rq"), 0xB, "queue element truncates");
    assert_eq!(u(&sim, "rd"), 0xD, "dynamic-array element truncates");
}
