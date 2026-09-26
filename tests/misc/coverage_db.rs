//! The results file and exit status, end to end: an `illegal_bins` hit
//! fails a `--error-exit` run.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_cov_db_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn illegal_bin_hit_fails_error_exit() {
    let d = scratch("illegal");
    let f = d.join("t.sv");
    std::fs::write(
        &f,
        "module t;\n  bit [1:0] s;\n  covergroup cg; cp : coverpoint s { bins ok = {[0:2]}; \
         illegal_bins bad = {3}; } endgroup\n  cg c = new();\n  initial begin s = 3; c.sample(); end\n\
         endmodule\n",
    )
    .unwrap();
    let run = |extra: &[&str]| {
        Command::new(xezim())
            .env("XEZIM_COV_DB", "/dev/null")
            .args(extra)
            .arg(&f)
            .output()
            .expect("run xezim")
    };
    let plain = run(&[]);
    assert_eq!(plain.status.code(), Some(0));
    let strict = run(&["--error-exit"]);
    assert_eq!(strict.status.code(), Some(1));
    let text = String::from_utf8_lossy(&strict.stdout);
    assert!(
        text.contains("Illegal bin hit at value 3: cg.cp.bad"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
