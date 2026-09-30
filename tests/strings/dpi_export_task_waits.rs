//! A DPI-exported task called from C returns only when it has finished
//! (#204). Its statements run on the synchronous path, where only `#delay`
//! used to wait: `fork ... join` dropped the rest of the task body and
//! returned, `wait fork` did nothing, and `@(ev)` / `wait(...)` did not
//! block. Each mode is checked by the time the task returned at, not just
//! by whether its increment ran.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

fn run(dir: &Path, lib: &Path, sv: &str) -> String {
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    text
}

const SV: &str = r#"
`timescale 1ns/1ps
module top;
  event done_ev, done2;
  bit req = 0, req2 = 0;
  int sink = 0;
  initial forever begin @(posedge req);  req = 0;  #100; -> done_ev; end
  initial forever begin @(posedge req2); req2 = 0; #50;  -> done2;   end

  task automatic c_delay();    #1000; sink++; endtask
  task automatic c_event();    req = 1; wait (done_ev); sink++; endtask
  task automatic c_at();       req2 = 1; @(done2); sink++; endtask
  task automatic c_forkjoin(); fork #500; join sink++; endtask
  task automatic c_joinany();  fork #300; #700; join_any sink++; endtask
  task automatic c_waitfork(); fork begin #500; end join_none wait fork; sink++; endtask

  export "DPI-C" task c_delay;
  export "DPI-C" task c_event;
  export "DPI-C" task c_at;
  export "DPI-C" task c_forkjoin;
  export "DPI-C" task c_joinany;
  export "DPI-C" task c_waitfork;
  import "DPI-C" function void from_c(int which);

  initial begin
    for (int m = 0; m < 6; m++) begin
      from_c(m);
      $display("R|mode %0d t=%0t sink=%0d", m, $time, sink);
      sink = 0;
    end
    $finish;
  end
endmodule
"#;

const C: &str = r#"
extern void c_delay(void), c_event(void), c_at(void);
extern void c_forkjoin(void), c_joinany(void), c_waitfork(void);
void from_c(int which) {
    switch (which) {
        case 0: c_delay(); break;
        case 1: c_event(); break;
        case 2: c_at(); break;
        case 3: c_forkjoin(); break;
        case 4: c_joinany(); break;
        case 5: c_waitfork(); break;
    }
}
"#;

#[test]
fn exported_tasks_called_from_c_block_until_they_finish() {
    let d = scratch("dpi_export_waits");
    let lib = shared_lib(&d, "from_c", C);
    let text = run(&d, &lib, SV);
    let _ = std::fs::remove_dir_all(&d);
    for line in [
        "R|mode 0 t=1000000 sink=1", // #1000
        "R|mode 1 t=1100000 sink=1", // wait(ev): +100
        "R|mode 2 t=1150000 sink=1", // @(ev): +50
        "R|mode 3 t=1650000 sink=1", // fork ... join: +500
        "R|mode 4 t=1950000 sink=1", // fork ... join_any: +300
        "R|mode 5 t=2450000 sink=1", // wait fork: also waits for mode 4's #700 child
    ] {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
}
