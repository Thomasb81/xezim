//! Commercial gate-level-sim flags: `+nospecify` suppresses specify-block
//! module path delays (zero-delay GLS); `+notimingcheck`/`+notimingchecks`
//! are accepted as documented no-ops (xezim does not model specify timing
//! checks, so they are permanently "disabled" already). Xcelium's `-`
//! spellings are accepted for both. CLI-level tests because the switch is a
//! process-global set by argument parsing.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

const SRC: &str = "`timescale 1ns/1ns
module buf1(input a, output y);
  assign y = a;
  specify (a => y) = 10; endspecify
endmodule
module tb;
  reg a = 0; wire y;
  buf1 u(.a(a), .y(y));
  initial begin
    #5 a = 1;
    #2 $display(\"MID y=%b\", y);
    #10 $display(\"END y=%b\", y);
    $finish;
  end
endmodule
";

fn run(args: &[&str]) -> String {
    let dir = std::env::temp_dir().join("xezim_specify_flags");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("sp.sv");
    std::fs::write(&sv, SRC).unwrap();
    let out = Command::new(xezim_bin())
        .args(args)
        .arg(&sv)
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Default: the (a => y) = 10 path delay holds y at 0 two ns after the input
/// edge, and it arrives by the end.
#[test]
fn specify_path_delay_applies_by_default() {
    let out = run(&[]);
    assert!(out.contains("MID y=0"), "path delay must defer y:\n{}", out);
    assert!(out.contains("END y=1"), "y must eventually arrive:\n{}", out);
}

/// `+nospecify` (and Xcelium's `-nospecify`): zero-delay — y flips immediately.
#[test]
fn nospecify_suppresses_path_delays() {
    for flag in ["+nospecify", "-nospecify"] {
        let out = run(&[flag]);
        assert!(
            out.contains("MID y=1"),
            "{} must suppress the specify path delay:\n{}",
            flag,
            out
        );
    }
}

/// The timing-check disables are recognized no-ops — no unknown-flag warning,
/// simulation unchanged.
#[test]
fn notimingcheck_is_a_quiet_noop() {
    for flag in ["+notimingcheck", "+notimingchecks", "-notimingchecks"] {
        let out = run(&[flag]);
        assert!(
            !out.to_lowercase().contains("unknown flag"),
            "{} must be recognized:\n{}",
            flag,
            out
        );
        assert!(out.contains("MID y=0"), "{} must not change timing:\n{}", flag, out);
    }
}
