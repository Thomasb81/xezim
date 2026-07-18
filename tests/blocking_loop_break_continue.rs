//! IEEE 1800-2023 §9.3.3: `break`/`continue` inside a blocking loop body
//! (`while`/`do…while`/`for` whose body has `#delay`/`wait`/`@event`) must
//! be honoured even though the loop body suspends and resumes via the
//! process-statement continuation model. Previously a `break` inside a
//! blocking `while` was silently ignored — the loop re-ran its body
//! indefinitely (the unrolled continuation re-appended the `while` stmt
//! without checking the loop-control flags the body set).
//!
//! Reference (commercial reference simulator): a `while` with `#5` + `break`
//! at i==4 produces `log == {1, 3}` (iter 2 `continue`-skipped, iter 4 broke).
use std::process::Command;

fn xezim() -> String {
    let base = env!("CARGO_MANIFEST_DIR");
    format!("{}/target/release/xezim", base)
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/blkbrk_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn blocking_while_honours_break_and_continue() {
    let src = r#"module top;
  int log[$];
  int i;
  initial begin
    i = 0;
    while (i < 10) begin
      #5;                 // blocking — loop body suspends
      i++;
      if (i == 2) continue;   // skip logging i==2
      if (i == 4) break;      // stop at i==4
      log.push_back(i);
    end
    if (log.size()==2 && log[0]==1 && log[1]==3)
      $display("RESULT PASS"); else $display("RESULT FAIL log=%p", log);
    $finish;
  end
endmodule
"#;
    let out = run(src, "while");
    assert!(out.contains("RESULT PASS"), "expected break/continue honoured\n{out}");
}

#[test]
fn blocking_for_honours_break() {
    // `for` is lowered to `while` — same gate applies.
    let src = r#"module top;
  int seen[$];
  initial begin
    for (int i = 0; i < 10; i++) begin
      #1;                 // blocking
      if (i == 5) break;
      seen.push_back(i);
    end
    // seen should be {0,1,2,3,4} — break at i==5
    if (seen.size()==5 && seen[0]==0 && seen[4]==4)
      $display("RESULT PASS"); else $display("RESULT FAIL seen=%p", seen);
    $finish;
  end
endmodule
"#;
    let out = run(src, "for");
    assert!(out.contains("RESULT PASS"), "expected for-break honoured\n{out}");
}
