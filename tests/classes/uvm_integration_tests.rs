//! UVM integration tests against the real Accellera UVM library.
//!
//! Requirement: a checkout of https://github.com/nitronis/UVM — one repo
//! carrying the four UVM releases as subdirectories (1.1d, 1.2,
//! 1800.2-2017, 1800.2-2020). Located in this order:
//!   1. `XEZIM_UVM_DIR` (points at the checkout root)
//!   2. `../UVM` (sibling of this repo)
//!   3. auto-cloned (shallow) into `target/uvm-checkout`
//!
//! The bare run-phase test is the #109 regression pin: `run_test()` used
//! to spin at 99% CPU without advancing time; it now completes the phase
//! cycle, so an objection held for #100 must end the run at t=100 on all
//! three validated UVM versions (1.1d bootstraps but is not yet
//! label-clean, see debug_notes round 44).

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

// Full driver/sequencer/monitor/scoreboard bench: build phase succeeds but
// the connect phase reports "connection count of 0" for every TLM export
// (seq_item_export, analysis fifos), then BUILDERR stops the run — port
// connect() registration is not reaching resolve_bindings yet.
#[test]
#[ignore = "uvm-1.2: TLM export connection counts resolve to 0 in the connect phase"]
fn test_uvm_complete() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_complete_test.sv")
        .expect("Could not read uvm_complete_test.sv");
    let sim = run_uvm("1.2", &[], test_src, "top").expect("UVM complete test failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        !out.iter().any(|l| l.contains("UVM_ERROR") || l.contains("UVM_FATAL")),
        "UVM errors:\n{}",
        out.join("\n")
    );
}

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
