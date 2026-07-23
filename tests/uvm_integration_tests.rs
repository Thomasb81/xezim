use std::fs;
use std::path::Path;
use std::process::Command;
use xezim::*;

#[test]
fn test_uvm_mock() {
    let src = fs::read_to_string("tests/uvm/uvm_simple_test.sv")
        .expect("Could not read uvm_simple_test.sv");

    // We need to provide the include directory for uvm_mock.svh
    let include_dirs = vec!["tests/uvm".to_string()];

    let res = simulate_multi(
        &[src],
        1000,
        Some("top"),
        &include_dirs,
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
    );

    assert!(res.is_ok(), "UVM Mock test failed: {:?}", res.err());
}

/// `uvm_analysis_imp_decl`-style routing: `in_ap.write(7)` must reach
/// `scb.write_in()`, not `scb.write()`. Requires `PURE_SV_LRM=0` so TLM
/// interception is active. Regression for the flat-ident dispatch bug where
/// statement-position `obj.connect(p)` never reached the TLM intercept.
#[test]
fn test_uvm_analysis_imp_decl() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let sv_file = Path::new(manifest_dir).join("tests/uvm/analysis_imp_decl_test.sv");
    let inc_dir = Path::new(manifest_dir).join("tests/uvm");

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("PURE_SV_LRM", "0")
        .arg("-I")
        .arg(inc_dir.to_str().unwrap())
        .arg(sv_file.to_str().unwrap())
        .output()
        .expect("failed to run xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("TEST_PASS") && !stdout.contains("TEST_FAIL"),
        "analysis_imp_decl routing test failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

// Real uvm-1.2 package: elaborates cleanly and the run_test/coreservice/
// factory bootstrap executes, but uvm_root construction (the phase
// scheduler) currently does not terminate. Ignored until the phasing
// engine runs to completion — running it would hang the test suite.
#[test]
#[ignore = "real uvm-1.2: uvm_root phase-scheduler does not yet terminate"]
fn test_uvm_complete() {
    let uvm_pkg = fs::read_to_string("uvm-1.2/src/uvm_pkg.sv").expect("Could not read uvm_pkg.sv");
    let test_src = fs::read_to_string("tests/uvm/uvm_complete_test.sv")
        .expect("Could not read uvm_complete_test.sv");

    let include_dirs = vec!["uvm-1.2/src".to_string()];

    // UVM needs UVM_NO_DPI if we don't have the DPI library
    let defines = vec![("UVM_NO_DPI".to_string(), None)];

    let res = simulate_multi(
        &[uvm_pkg, test_src],
        2000,
        Some("top"),
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
    );

    assert!(res.is_ok(), "UVM Complete test failed: {:?}", res.err());
}

#[test]
#[ignore = "real uvm-1.2: uvm_root phase-scheduler does not yet terminate"]
fn test_uvm_hello_world() {
    let uvm_pkg = fs::read_to_string("uvm-1.2/src/uvm_pkg.sv").expect("Could not read uvm_pkg.sv");
    let test_src = fs::read_to_string("uvm-1.2/examples/simple/hello_world/hello_world.sv")
        .expect("Could not read hello_world.sv");

    let include_dirs = vec![
        "uvm-1.2/src".to_string(),
        "uvm-1.2/examples/simple/hello_world".to_string(),
    ];

    let defines = vec![("UVM_NO_DPI".to_string(), None)];

    let res = simulate_multi(
        &[uvm_pkg, test_src],
        10000,
        Some("hello_world"),
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
    );

    assert!(res.is_ok(), "UVM Hello World test failed: {:?}", res.err());
}
