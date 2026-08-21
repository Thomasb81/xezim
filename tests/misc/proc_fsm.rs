//! Roadmap steps 11-12 (opt-in XEZIM_PROC_FSM=1): blocking always bodies
//! compile into bytecode FSMs with wait insns; a resume re-enters at the
//! saved pc with per-process registers instead of re-walking the AST chain.
//!
//! The multi-wait expected values below are REFERENCE-VERIFIED: a body that
//! consumes time past the next clock edge MISSES that edge (§9.2.2 process
//! semantics — the process is not at its event control). xezim's legacy
//! edge path fires such bodies on every posedge instead; the FSM is the
//! conforming behavior, which is why the values differ from a plain run.

use std::process::Command;

fn run_fsm(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_proc_fsm_{}_{}", tag, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("t.sv");
    std::fs::write(&f, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "tb", "--max-time", "1000"])
        .arg(&f)
        .env("XEZIM_PROC_FSM", "1")
        .env("XEZIM_PROC_LOOP_STATS", "1")
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn fsm_multiwait_bodies_match_reference() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  reg clk = 0;
  reg [7:0] cnt = 0, mirror = 0, tick = 0;
  always begin
    #5 clk = ~clk;
  end
  always @(posedge clk) begin
    cnt = cnt + 1;
    #2;
    mirror = cnt ^ 8'h0f;
    repeat (2) @(negedge clk);
    mirror = mirror + 1;
  end
  always begin
    @(posedge clk);
    tick <= tick + 3;
    #1 tick <= tick + 1;
  end
  initial begin
    #103 $display("R cnt=%0d mirror=%0d tick=%0d clk=%b", cnt, mirror, tick, clk);
    #40 $display("R2 cnt=%0d mirror=%0d tick=%0d", cnt, mirror, tick);
    $finish;
  end
endmodule
"#;
    let text = run_fsm(src, "multiwait");
    assert!(
        text.contains("[PROC-FSM] registered"),
        "FSM must engage:\n{}",
        text
    );
    // Reference-simulator values (edge missed while the body is mid-flight).
    assert!(
        text.contains("R cnt=5 mirror=11 tick=40 clk=0"),
        "multi-wait always semantics:\n{}",
        text
    );
    assert!(
        text.contains("R2 cnt=7 mirror=9 tick=56"),
        "later window:\n{}",
        text
    );
}
