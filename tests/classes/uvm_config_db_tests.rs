//! config_db scope-matching regression tests (pure in-process).
//!
//! These exercise xezim's `uvm_config_db#(T)::set/get` interception
//! (scope-aware instance-name matching, wildcards, and misses) entirely
//! in-process via `simulate_multi` — no subprocess, no external UVM library,
//! no reference simulator.
//!
//! The interception fires once the parser resolves `uvm_config_db#(int)` as a
//! defined parameterized class, so instead of pulling in the full 1800.2
//! library (which is not shipped in this repo and is absent in CI) we prepend a
//! minimal stub: a `uvm_component` shell that answers `get_full_name()`, and a
//! `uvm_config_db` shell whose empty static methods xezim intercepts by name.

use xezim::simulate_multi;

/// Minimal UVM class shells. `uvm_config_db`'s static `set`/`get`/`exists`
/// bodies are empty because xezim intercepts them by class name — the real
/// logic lives in `Simulator::exec_config_db`. `uvm_component` only needs to
/// satisfy `get_full_name()` for the interception's scope resolution.
const STUB: &str = r#"
class uvm_component;
  string m_name;
  function new(string name); m_name = name; endfunction
  function string get_full_name(); return m_name; endfunction
endclass

class uvm_config_db #(type T = int);
  static function void set(uvm_component cntxt, string inst, string field, T val); endfunction
  static function bit get(uvm_component cntxt, string inst, string field, ref T val); endfunction
  static function bit exists(uvm_component cntxt, string inst, string field); endfunction
endclass
"#;

/// Run `src` (a `module top`) in-process and return the joined `$display`
/// output. The UVM stub is prepended so the interception recognises
/// `uvm_config_db`; no external files are read.
fn run_in_process(src: &str) -> String {
    let full = format!("{}\n{}", STUB, src);
    let sim = simulate_multi(
        &[full],
        50_000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("simulation failed");

    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Specific instance name: a `get` whose context/inst_name matches a prior `set`
/// succeeds; a wildcard `set(null, "*", ...)` matches any getter.
#[test]
fn test_config_db_inst_name() {
    let src = r#"
module top;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "tc", "my_int", 42);
    #1;
    uvm_component comp1 = new("tc");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp1, "tc", "my_int", val))
      $display("GET1_OK: %0d", val);
    else
      $display("GET1_FAIL");

    // Wildcard should match any getter path.
    #1;
    uvm_config_db#(int)::set(null, "*", "wild_int", 99);
    #1;
    if (uvm_config_db#(int)::get(comp1, "any", "wild_int", val))
      $display("GET3_OK: %0d", val);
    else
      $display("GET3_FAIL: wildcard should match");

    $finish;
  end
endmodule
"#;
    let out = run_in_process(src);
    println!("{}", out);
    assert!(out.contains("GET1_OK: 42"), "same-instance get should hit: {}", out);
    assert!(out.contains("GET3_OK: 99"), "wildcard get should hit: {}", out);
}

/// A wildcard `set(null, "*", field, v)` is visible to any getter, and the
/// retrieved value is the one that was set.
#[test]
fn test_config_db_wildcard() {
    let src = r#"
module top;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "*", "my_int", 99);
    #1;
    uvm_component comp = new("comp");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp, "any_path", "my_int", val)) begin
      if (val == 99)
        $display("TEST_PASS");
      else
        $display("TEST_FAIL: expected 99 got %0d", val);
    end else begin
      $display("TEST_FAIL: get returned 0");
    end
    $finish;
  end
endmodule
"#;
    let out = run_in_process(src);
    println!("{}", out);
    assert!(out.contains("TEST_PASS"), "wildcard value should round-trip: {}", out);
}

/// A specific-instance set hits its getter; a wildcard set hits any getter; a
/// field that was never set misses.
#[test]
fn test_config_db_hit_wildcard_and_miss() {
    let src = r#"
module top;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "tc", "my_int", 42);
    #1;
    uvm_component comp = new("comp");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp, "tc", "my_int", val))
      $display("T1_GET: %0d", val);
    else
      $display("T1_FAIL");

    #1;
    uvm_config_db#(int)::set(null, "*", "wild_int", 77);
    #1;
    if (uvm_config_db#(int)::get(comp, "any", "wild_int", val))
      $display("T2_GET: %0d", val);
    else
      $display("T2_FAIL");

    #1;
    if (uvm_config_db#(int)::get(comp, "*", "nonexist", val))
      $display("T3_FAIL: should not exist");
    else
      $display("T3_OK: not found as expected");

    $finish;
  end
endmodule
"#;
    let out = run_in_process(src);
    println!("{}", out);
    assert!(out.contains("T1_GET: 42"), "specific-instance get: {}", out);
    assert!(out.contains("T2_GET: 77"), "wildcard get: {}", out);
    assert!(out.contains("T3_OK"), "unset field should miss: {}", out);
}
