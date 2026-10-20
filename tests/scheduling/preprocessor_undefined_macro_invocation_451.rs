//! ISSUE #451 (regression): an undefined text-macro *invocation*
//! (`` `NAME(a, b) ``) must not cascade into a wall of derivative parse
//! errors. The whole balanced `(...)` argument list is collapsed by the
//! preprocessor into a single inert literal token, so the design parses with
//! just the one undefined-macro warning and no error.

use std::process::Command;

fn xezim_bin() -> &'static str {
    env!("CARGO_BIN_EXE_xezim")
}

fn temp_source(body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let unique = format!(
        "xezim-pp-451-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let source = dir.join("i451.sv");
    std::fs::write(&source, body).expect("write source");
    (dir, source)
}

#[test]
fn undefined_macro_invocation_parses_with_zero_errors() {
    let (dir, source) = temp_source(
        "`define FOO 1\n\
         module m;\n\
         int x;\n\
         initial begin\n\
           `UNDEFINED_MACRO(1, 2)\n\
           x = 1;\n\
           $display(\"done\");\n\
         end\n\
         endmodule\n",
    );
    let out = Command::new(xezim_bin())
        .arg("--parse")
        .arg(&source)
        .output()
        .expect("run parser");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Success: the undefined invocation collapses, the design still parses.
    assert!(
        out.status.success(),
        "expected a clean parse:\n{stderr}"
    );
    // The real problem is still reported, once.
    assert!(
        stderr.contains("macro `UNDEFINED_MACRO is undefined"),
        "missing undefined-macro warning:\n{stderr}"
    );
    // And no derivative errors from the swallowed argument list.
    assert!(
        !stderr.contains("error: expected"),
        "cascade of derivative errors must be gone:\n{stderr}"
    );
    std::fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn bare_undefined_macro_also_parses_zero_errors() {
    let (dir, source) = temp_source(
        "`define FOO 1\n\
         module m;\n\
         int x;\n\
         initial begin\n\
           `UNDEFINED_MACRO\n\
           x = 1;\n\
         end\n\
         endmodule\n",
    );
    let out = Command::new(xezim_bin())
        .arg("--parse")
        .arg(&source)
        .output()
        .expect("run parser");
    assert!(
        out.status.success(),
        "bare undefined macro should parse cleanly:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(dir).expect("remove temp directory");
}