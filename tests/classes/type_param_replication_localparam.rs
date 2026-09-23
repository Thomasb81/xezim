//! A localparam in a type-parameterized class whose replication count comes
//! from `$bits` of the type parameter (`{$bits(DT) - 1{1'b1}}`). The
//! reference simulator elaborates it and runs; xezim never finishes (the
//! sv-tests `class_test_52` shape). Run through the binary under a
//! watchdog so a hang fails the test instead of stalling the suite.

use std::process::Command;
use std::time::{Duration, Instant};

#[test]
#[ignore = "elaboration never finishes on a $bits-sized replication in a type-parameterized class (fix pending)"]
fn bits_of_type_parameter_as_replication_count_elaborates() {
    let dir = std::env::temp_dir().join(format!("xezim_type_param_repl_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("repl.sv");
    std::fs::write(
        &path,
        "class base; endclass\n\
         class how_wide #(type DT=int) extends base;\n\
         \x20 localparam Max_int = {$bits(DT) - 1{1'b1}};\n\
         endclass\n\
         module tb;\n\
         \x20 initial $display(\"DONE\");\n\
         endmodule\n",
    )
    .expect("write design");
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", "--no-cache", path.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run xezim");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut s = String::new();
            use std::io::Read;
            child.stdout.take().unwrap().read_to_string(&mut s).ok();
            assert!(status.success() && s.contains("DONE"), "run failed:\n{s}");
            return;
        }
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("xezim did not finish within 20 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
