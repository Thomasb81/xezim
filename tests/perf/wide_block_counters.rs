//! PERFORMANCE REGRESSION guard for the WIDE two-state class — blocks whose
//! registers are wider than 128 bits (up to 512) — which runs on the
//! two-state executor by default (`XEZIM_TS_WIDE512=0` restores the old
//! interpreter routing). Asserts on deterministic WORK COUNTERS, like
//! `work_counters.rs`: each shape must be admitted (`two_state_evals` per
//! clock) and produce the four-state interpreter's answer.
//!
//! The shapes are the ones that kept such blocks interpreted on a C910 SoC:
//! a wide read-modify-write in one block (`acc = {acc[..], ..}`), a mux
//! between two wide values, a wide `'x` reset default, and a plain wide
//! bitwise entry. Bounds are CEILINGS/FLOORS; re-baseline when an
//! optimization moves them.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_wide_block_counters_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).expect("write temporary design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", "--no-cache", path.to_str().unwrap()])
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "run failed:\n{text}");
    text
}

fn stat(text: &str, key: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.contains(key))
        .unwrap_or_else(|| panic!("missing counter `{key}`:\n{text}"));
    let after = &line[line.find(key).unwrap() + key.len()..];
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("unparsable counter `{key}` in `{line}`"))
}

/// A blocking read-modify-write of a 336-bit register in one clocked block.
/// The block reads `acc` and then overwrites it, so a two-state bail after
/// the write would re-run against the new value; the wide value is saved
/// at entry (`SaveSigW`) instead of the whole block bailing (it used to
/// bail with `RawHazard`).
#[test]
fn wide_read_modify_write_block_is_two_state() {
    let text = run(
        "wide_rmw",
        r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [335:0] acc; logic [43:0] tag; int cyc = 0;
  always @(posedge clk) begin
    acc = {acc[291:0], acc[335:292] ^ tag};
    tag = tag + 44'd7;
    cyc <= cyc + 1;
  end
  initial begin
    acc = {8{42'h2_5a5a_5a5a_5}}; tag = 44'h1;
    repeat (40) @(posedge clk);
    #1 $display("RMW %h %h %0d", acc[63:0], acc[335:272], cyc);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("RMW 5913d9f059105c58 5206d886ee9465a1 40"), "answer:\n{text}");
    assert!(stat(&text, "two_state_evals=") >= 40, "the wide RMW block left two-state:\n{text}");
}

/// A 214-bit mux (`WSel`) in a combinational entry.
#[test]
fn wide_mux_entry_is_two_state() {
    let text = run(
        "wide_mux",
        r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic sel = 0; logic [213:0] a, b, y; int cyc = 0;
  always_comb y = sel ? a : b;
  always @(posedge clk) begin sel <= ~sel; a <= a + 214'd3; b <= b - 214'd1; cyc <= cyc + 1; end
  initial begin
    a = 214'h10; b = 214'hf000;
    repeat (40) @(posedge clk);
    #1 $display("MUX %h %h %0d", y[63:0], y[213:150], cyc);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("MUX 000000000000efd8 0000000000000000 40"), "answer:\n{text}");
    assert!(stat(&text, "two_state_evals=") >= 40, "the wide mux entry left two-state:\n{text}");
}

/// A wide `'x` reset default (`{271{1'bx}}`) stored from a combinational
/// entry: an x fill of both planes (`RangeFillXW`) rather than a bail.
#[test]
fn wide_x_fill_entry_is_two_state() {
    let text = run(
        "wide_xfill",
        r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic en = 0; logic [270:0] d, q; int cyc = 0;
  always_comb if (en) q = d; else q = {271{1'bx}};
  always @(posedge clk) begin en <= ~en; d <= d + 271'd5; cyc <= cyc + 1; end
  initial begin
    d = 271'h77;
    repeat (40) @(posedge clk);
    #1 $display("XF %b %h %0d", q[270], q[15:0], cyc);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("XF x xxxx 40"), "answer:\n{text}");
    assert!(stat(&text, "two_state_evals=") >= 40, "the x-fill entry left two-state:\n{text}");
}

/// A plain 300-bit bitwise entry: the wide class itself.
#[test]
fn wide_bitwise_entry_is_two_state() {
    let text = run(
        "wide_class",
        r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [299:0] a, b, y; int cyc = 0;
  always_comb y = (a & b) ^ {b[149:0], a[299:150]};
  always @(posedge clk) begin a <= {a[298:0], a[299]}; b <= b + 300'd11; cyc <= cyc + 1; end
  initial begin
    a = {5{60'h1234_5678_9abc_def}}; b = {5{60'hf0f0_0ff0_ff00_0f0f}};
    repeat (40) @(posedge clk);
    #1 $display("WC %h %h %0d", y[63:0], y[299:236], cyc);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("WC 2dd591369b37acca 0000003c3c03fc3f 40"), "answer:\n{text}");
    assert!(stat(&text, "two_state_evals=") >= 40, "the wide entry left two-state:\n{text}");
}
