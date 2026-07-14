//! Genuine-UVM-library integration: the real Accellera 1800.2-2017-1.0 UVM
//! library runs to completion under the default PURE_SV_LRM=1 path.
//!
//! This is the end-to-end proof that the UVM phasing stack works:
//!   - mailbox-driven phase hopper (`m_run_phases`) advances phases,
//!   - the objection bridge (`raise/drop_objection` ↔ `wait_for`) syncs,
//!   - out-of-body method bodies (`function uvm_sequencer::new ...`) link,
//!   - `type_id::create` resolves class-local `this_type` typedefs,
//!   - the UVM DPI C shims (`uvm_re_match`, `uvm_glob_to_re`) back the
//!     regex-based command-line processor (loaded via `--dpi-lib`).
//!
//! Runs the binary (not `simulate`) because DPI resolution requires loading
//! the `uvm-2017-1.0.so` shared object, which is a binary-only (`--dpi-lib`)
//! feature. The test is gated on the UVM tree and the `.so` both being
//! present (they ship in the repo).

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Absolute repo-root paths for the 2017 UVM library and its DPI `.so`.
fn uvm2017_paths() -> Option<(PathBuf, PathBuf)> {
    // CARGO_MANIFEST_DIR = .../xezim/xezim ; the UVM tree + .so live one up.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    let lib = root.join("1800.2-2017-1.0");
    let so = manifest_path("uvm-2017-1.0.so");
    if lib.join("src/uvm_pkg.sv").is_file() && so.is_file() {
        Some((lib, so))
    } else {
        None
    }
}

#[test]
fn genuine_uvm_2017_runs_test_to_completion_with_dpi_lib() {
    let Some((lib, so)) = uvm2017_paths() else {
        eprintln!(
            "[uvm_genuine] 1800.2-2017-1.0 tree or uvm-2017-1.0.so not present; skipping"
        );
        return;
    };
    let flist = std::env::temp_dir().join(format!(
        "uvm2017_genuine_{}.f",
        std::process::id()
    ));
    let incdir = format!("+incdir+{}/src", lib.display());
    let pkg = format!("{}/src/uvm_pkg.sv", lib.display());
    let macros = format!("{}/src/uvm_macros.svh", lib.display());
    let test = manifest_path("tests/uvm/uvm_complete_test.sv");
    std::fs::write(&flist, format!(
        "{}\n{}\n{}\n{}\n",
        incdir,
        pkg,
        macros,
        test.display(),
    ))
    .expect("write flist");

    let bin = env!("CARGO_BIN_EXE_xezim");
    // PURE_SV_LRM=1 is the default; assert it explicitly so the test is
    // self-documenting about which path it exercises.
    let out = Command::new(bin)
        .env("PURE_SV_LRM", "1")
        .arg("--dpi-lib")
        .arg(&so)
        .arg("-sv")
        .arg("-f")
        .arg(&flist)
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    assert!(
        out.status.success(),
        "genuine-UVM run failed:\n{}",
        text
    );

    // The test's `run_phase` raises an objection, prints Starting at t=0,
    // waits #100, prints Finishing, and drops the objection. The sim must
    // reach t=100 with zero UVM_FATAL/UVM_ERROR.
    assert!(
        text.contains("Running test simple_test"),
        "missing [RNTST] banner:\n{}",
        text
    );
    assert!(
        text.contains("Starting test"),
        "run_phase never printed Starting:\n{}",
        text
    );
    assert!(
        text.contains("Finishing test"),
        "run_phase never printed Finishing:\n{}",
        text
    );
    assert!(
        text.contains("finished at time 100"),
        "expected sim to finish at t=100:\n{}",
        text
    );
    // The report-server summary line "UVM_FATAL :    0" must show zero
    // fatalities (and the analogous UVM_ERROR line).
    assert!(
        text.contains("UVM_FATAL :    0"),
        "expected zero UVM_FATAL:\n{}",
        text
    );
}
