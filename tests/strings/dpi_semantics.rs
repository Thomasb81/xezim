//! DPI semantics beyond type mapping (`dpi_integration_tests` covers the
//! argument and return types): imported TASKS that consume time through an
//! exported task, the scope a context import runs in, C -> SV -> C
//! recursion, `pure` imports in continuous logic, the `c_name =` link form,
//! a C-owned object carried as a chandle, and one call writing outputs of
//! several types. Each bench prints `T|` lines; the expected values were
//! worked out by hand from IEEE 1800-2017 clause 35.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Build `c_file` into a fresh shared library.
fn compile(c_file: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = Path::new(c_file).file_stem().unwrap().to_string_lossy();
    let so = std::env::temp_dir().join(format!("{}_{}_{}.so", stem, std::process::id(), nanos));
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(manifest_path("include"))
        .arg(manifest_path(c_file))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed for {c_file}");
    so
}

/// Run `sv_file` against the library and return its `T|` lines.
fn tagged(so: &Path, sv_file: &str) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(so)
        .args(["--max-time", "1000"])
        .arg(manifest_path(sv_file))
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "xezim failed for {sv_file}:\n{text}");
    let _ = std::fs::remove_file(so);
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

/// §35.5.2/§35.6.1: an imported task consumes time through an exported task
/// that delays; its outputs are written when it returns; a second call after
/// the first returned runs from the current time; a task that never waits
/// returns at once. `svGetTime` reads the simulation time from C.
#[test]
fn dpi_imported_task_consumes_time() {
    let so = compile("tests/dpi/dpi_tasks.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_tasks_test.sv"),
        [
            "T|instant d=42 t=0",
            "T|step who=0 step=0 t=5",
            "T|step who=0 step=1 t=10",
            "T|step who=0 step=2 t=15",
            "T|solo started=0 elapsed=15 t=15",
            "T|step who=1 step=0 t=22",
            "T|step who=1 step=1 t=29",
            "T|again started=15 elapsed=14 t=29",
            "T|done",
        ]
    );
}

/// §35.5.2: two processes inside the same imported task at once, each
/// waiting on its own period, interleave and each returns when its own
/// waits are done.
///
/// Ignored: xezim runs an exported task's delay by nesting the scheduler
/// inside the C call, so the second caller's C frame sits above the first's
/// and the first cannot return until the second has: the 7-cycle worker's
/// steps land at 9 and 16 instead of 7 and 14. Running each imported-task
/// call on its own stack would lift this.
#[test]
#[ignore = "concurrent imported tasks unwind last-in first-out (nested scheduler)"]
fn dpi_concurrent_imported_tasks_interleave() {
    let so = compile("tests/dpi/dpi_tasks.c");
    let mut out = tagged(&so, "tests/dpi/dpi_tasks_concurrent_test.sv");
    // Steps at the same time come from different processes; compare them
    // without their same-time order.
    out.sort();
    let mut want = vec![
        "T|step who=2 step=0 t=3",
        "T|step who=2 step=1 t=6",
        "T|step who=1 step=0 t=7",
        "T|step who=2 step=2 t=9",
        "T|step who=1 step=1 t=14",
        "T|pair w1 started=0 elapsed=14 w2 started=0 elapsed=9 t=14",
        "T|done",
    ];
    want.sort();
    assert_eq!(out, want);
}

/// §35.5.3/§36.6: a context import runs in the scope of the instance it is
/// called through (`ua.c_fn()`) or from (an `initial` inside the
/// instance) — named as `%m` names it — and a top-level one in `top`;
/// `svGetScopeFromName` and `svGetNameFromScope` round-trip; `svSetScope`
/// returns the scope it replaces and restores it; and a context import
/// recurses through an exported function (C -> SV -> C, 10 levels).
#[test]
fn dpi_context_scope_and_recursion() {
    let so = compile("tests/dpi/dpi_scope_exports.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_scope_exports_test.sv"),
        [
            "T|names a=top.ua b=top.ub",
            "T|top=top",
            "T|lookup top.ua | top.ub | top",
            "T|switch in=top.ub prev=top.ua back=top.ua",
            "T|fact 1=1 5=120 10=3628800",
            "T|self top.ua=top.ua",
            "T|self top.ub=top.ub",
            "T|done",
        ]
    );
}

/// §35.5.3: a function exported from an INSTANTIATED module is called, from
/// a context import, in the instance the import was called through.
///
/// Ignored: exports are registered by bare name at elaboration, and not
/// from instantiated modules — the library's (weak) reference to the export
/// stays unresolved, and a strong one fails to load (`undefined symbol`).
#[test]
#[ignore = "exports from instantiated modules are not registered"]
fn dpi_export_from_instantiated_module() {
    let so = compile("tests/dpi/dpi_scope_exports.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_instance_exports_test.sv"),
        ["T|ids a=11 b=22", "T|done"]
    );
}

/// §35.5.1-35.5.4: a `pure` import in a continuous assignment and an
/// always_comb block follows its input; two SV names linked to one C symbol
/// with `c_name = function ...`; a C-owned counter held as a chandle by a
/// class; and one call writing real, string, bit-vector, logic-vector (with
/// an X bit) and bit outputs.
#[test]
fn dpi_pure_alias_chandle_and_outputs() {
    let so = compile("tests/dpi/dpi_pure_alias.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_pure_alias_test.sv"),
        [
            "T|pure v=0 wire=0 comb=1",
            "T|pure v=7 wire=1 comb=1",
            "T|pure v=6 wire=0 comb=1",
            "T|pure called=1",
            "T|alias 5 42",
            "T|chandle a=2 b=20",
            "T|released 1",
            "T|outputs half=18.5 label=x=37 byte=37 nib=0101 odd=1",
            "T|outputs half=10.5 label=x=21 byte=21 nib=010x odd=1",
            "T|done",
        ]
    );
}
