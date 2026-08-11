//! UVM integration tests against the real Accellera UVM library.
//!
//! Requirement: a checkout of https://github.com/nitronis/UVM — one repo
//! carrying the four UVM releases as subdirectories (1.1d, 1.2,
//! 1800.2-2017, 1800.2-2020). Located in this order:
//!   1. `XEZIM_UVM_DIR` (points at the checkout root)
//!   2. `../UVM` (sibling of this repo)
//!   3. auto-cloned (shallow) into `target/uvm-checkout`
//!
//! The bare run-phase tests are the #109 regression pin: `run_test()` used
//! to spin at 99% CPU without advancing time; it now completes the phase
//! cycle, so an objection held for #100 must end the run at t=100 on all
//! three validated UVM versions (1.1d bootstraps but is not yet
//! label-clean, see debug_notes round 44). The 2020 tests additionally pin
//! the full TLM bench (sequencer/driver/monitor/scoreboard) and config_db.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use xezim::*;

fn uvm_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Ok(d) = std::env::var("XEZIM_UVM_DIR") {
            let p = PathBuf::from(d);
            assert!(
                p.join("1.2/src/uvm_pkg.sv").exists(),
                "XEZIM_UVM_DIR does not look like a nitronis/UVM checkout: {}",
                p.display()
            );
            return p;
        }
        let sibling = Path::new("../UVM");
        if sibling.join("1.2/src/uvm_pkg.sv").exists() {
            return sibling.canonicalize().expect("canonicalize ../UVM");
        }
        let target = Path::new("target/uvm-checkout");
        if !target.join("1.2/src/uvm_pkg.sv").exists() {
            let status = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "https://github.com/nitronis/UVM",
                    target.to_str().unwrap(),
                ])
                .status()
                .expect("git not available to fetch the UVM checkout");
            assert!(
                status.success(),
                "cloning https://github.com/nitronis/UVM failed \
                 (set XEZIM_UVM_DIR to an existing checkout to skip the clone)"
            );
        }
        target.canonicalize().expect("canonicalize target/uvm-checkout")
    })
    .clone()
}

/// Objection raised in run_phase, held #100, dropped: the simulation must
/// end at exactly t=100 (phase cycle completes, $finish fires).
const BARE_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class my_test extends uvm_test;
   `uvm_component_utils(my_test)
   function new(string name = "my_test", uvm_component parent = null);
      super.new(name, parent);
   endfunction
   task run_phase(uvm_phase phase);
      phase.raise_objection(this);
      #100;
      phase.drop_objection(this);
   endtask
endclass

module top;
   initial run_test("my_test");
endmodule
"#;

fn run_uvm(version: &str, extra_incdirs: &[String], test_src: String, top: &str) -> Result<compiler::Simulator, String> {
    let root = uvm_dir().join(version);
    let src_dir = root.join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv"))
        .unwrap_or_else(|e| panic!("read {}/src/uvm_pkg.sv: {}", version, e));
    let mut include_dirs = vec![src_dir.to_str().unwrap().to_string()];
    include_dirs.extend_from_slice(extra_incdirs);
    let defines = vec![("UVM_NO_DPI".to_string(), None)];
    simulate_multi(
        &[uvm_pkg, test_src],
        2000,
        Some(top),
        &include_dirs,
        &[],
        None,
        false,
        None,
        None,
        &defines,
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
}

fn bare_run_completes_on(version: &str) {
    let sim = run_uvm(version, &[], BARE_TEST.to_string(), "top")
        .unwrap_or_else(|e| panic!("UVM {} bare test failed to simulate: {}", version, e));
    assert_eq!(
        sim.time, 100,
        "UVM {}: run_phase objection held to #100 must end the run at t=100",
        version
    );
}

#[test]
fn uvm_1_2_bare_run_phase_completes() {
    bare_run_completes_on("1.2");
}

#[test]
fn uvm_1800_2_2017_bare_run_phase_completes() {
    bare_run_completes_on("1800.2-2017");
}

#[test]
fn uvm_1800_2_2020_bare_run_phase_completes() {
    bare_run_completes_on("1800.2-2020");
}

/// Full driver/sequencer/monitor/scoreboard bench on UVM 1800.2-2020: the
/// sequence drives 10 transactions through the sequencer/driver TLM
/// handshake; the monitor's analysis port fans out to the scoreboard. Pins
/// the assoc-key fix that unblocked TLM: connection registries are keyed by
/// hierarchical names (dotted strings), which key enumeration used to drop —
/// every `connect()` then resolved to 0 connections and BUILDERR aborted
/// the run at t=0.
#[test]
fn uvm_2020_complete_bench_runs_traffic() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_complete_test.sv")
        .expect("Could not read uvm_complete_test.sv");
    let sim = run_uvm("1800.2-2020", &[], test_src, "top")
        .expect("UVM 2020 complete bench failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        !out.iter().any(|l| l.contains("UVM_ERROR") || l.contains("UVM_FATAL")),
        "UVM errors:\n{}",
        out.join("\n")
    );
    let checked = out.iter().filter(|l| l.contains("[SB] Checked data:")).count();
    assert_eq!(checked, 10, "scoreboard must check all 10 transactions:\n{}", out.join("\n"));
    assert_eq!(sim.time, 100, "test drops its objection at t=100");
}

/// UVM 1800.2-2020 `uvm_config_db#(int)`: a value set at the top against
/// `uvm_test_top` must be visible to the test's build_phase get.
#[test]
fn uvm_2020_config_db_reaches_build_phase() {
    let sim = run_uvm("1800.2-2020", &[], CFG_TEST.to_string(), "top")
        .expect("UVM 2020 config_db test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("cfg=77")),
        "config_db set(uvm_test_top, magic, 77) must reach build_phase:\n{}",
        out.join("\n")
    );
}

/// `base_c::type_id::set_type_override(deriv_c::get_type())` must make
/// `type_id::create` return the derived type. Pins the unpacked-struct
/// class-property copy fix: the factory's override matcher copies
/// `override.orig` (a struct with a class-handle + string member) into a
/// local through a ?: — that whole-struct read through a frame-held handle
/// came back x, so every registered override failed to match.
#[test]
fn uvm_2020_factory_type_override() {
    let sim = run_uvm("1800.2-2020", &[], CFG_TEST.to_string(), "top")
        .expect("UVM 2020 factory test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("kind=deriv")),
        "type override must make create() return the derived type:\n{}",
        out.join("\n")
    );
}

/// Restoration of the shim-era `analysis_imp_decl_test.sv` (removed with the
/// PURE_SV_LRM=0 intercepts in 8bace9e without a native successor): two
/// `uvm_analysis_imp_decl` suffixed imps must route `port.write()` through
/// the macro-generated forwarders to the RIGHT suffixed method
/// (`write_in`/`write_out`). This is the requirement whose lost coverage let
/// the dotted-key enumeration regression (92abea7) break `connect()`
/// silently — now pinned against the real 1800.2-2020 library.
#[test]
fn uvm_2020_analysis_imp_decl_routes_to_suffixed_write() {
    let sim = run_uvm("1800.2-2020", &[], IMP_DECL_TEST.to_string(), "top")
        .expect("UVM 2020 imp_decl test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("TEST_PASS")),
        "imp_decl write routing must hit the suffixed subscribers (2/1):\n{}",
        out.join("\n")
    );
}

const IMP_DECL_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

`uvm_analysis_imp_decl(_in)
`uvm_analysis_imp_decl(_out)

module top;
  class scoreboard;
    int n_in, n_out;
    function void write_in(int t);  n_in++;  endfunction
    function void write_out(int t); n_out++; endfunction
  endclass

  initial begin
    automatic scoreboard scb = new;
    automatic uvm_analysis_imp_in  #(int, scoreboard) imp_in  = new("imp_in",  scb);
    automatic uvm_analysis_imp_out #(int, scoreboard) imp_out = new("imp_out", scb);
    automatic uvm_analysis_port #(int) in_ap  = new("in_ap");
    automatic uvm_analysis_port #(int) out_ap = new("out_ap");

    in_ap.connect(imp_in);
    out_ap.connect(imp_out);
    in_ap.resolve_bindings();
    out_ap.resolve_bindings();

    in_ap.write(7);
    in_ap.write(8);
    out_ap.write(9);

    if (scb.n_in !== 2 || scb.n_out !== 1)
      $display("TEST_FAIL: n_in=%0d (exp 2) n_out=%0d (exp 1)", scb.n_in, scb.n_out);
    else
      $display("TEST_PASS");
    $finish;
  end
endmodule
"#;

/// Factory-override + config_db probe (see the two tests above).
const CFG_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class base_c extends uvm_component;
  `uvm_component_utils(base_c)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "base"; endfunction
endclass

class deriv_c extends base_c;
  `uvm_component_utils(deriv_c)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "deriv"; endfunction
endclass

class cfg_test extends uvm_test;
  `uvm_component_utils(cfg_test)
  base_c comp;
  int cfgval;
  function new(string name = "cfg_test", uvm_component parent = null); super.new(name, parent); endfunction
  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    base_c::type_id::set_type_override(deriv_c::get_type());
    comp = base_c::type_id::create("comp", this);
    if (!uvm_config_db#(int)::get(this, "", "magic", cfgval)) cfgval = -1;
  endfunction
  virtual task run_phase(uvm_phase phase);
    phase.raise_objection(this);
    `uvm_info("CFG", $sformatf("kind=%s cfg=%0d", comp.kind(), cfgval), UVM_LOW)
    #10;
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial begin
    uvm_config_db#(int)::set(null, "uvm_test_top", "magic", 77);
    run_test("cfg_test");
  end
endmodule
"#;

// Phasing ends at t=0 with no producer/consumer traffic — the packet
// stimulus never starts. Same family as the connect-phase gap above.
#[test]
#[ignore = "uvm-1.2: hello_world terminates at t=0 without producer output"]
fn test_uvm_hello_world() {
    let hw_dir = uvm_dir().join("1.2/examples/simple/hello_world");
    let test_src = std::fs::read_to_string(hw_dir.join("hello_world.sv"))
        .expect("Could not read hello_world.sv");
    let incs = vec![hw_dir.to_str().unwrap().to_string()];
    let sim = run_uvm("1.2", &incs, test_src, "hello_world").expect("hello_world failed");
    assert!(sim.time > 0, "hello_world must advance past t=0");
}
