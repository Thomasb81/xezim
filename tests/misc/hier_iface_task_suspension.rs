//! Event controls inside tasks called HIERARCHICALLY — `u_m.t()`,
//! `u_if.t()`, or `vif.t()` through a virtual-interface class property —
//! ran on the synchronous path, whose `@(edge)` aborts the body instead of
//! suspending: a monitor BFM's `@(negedge clk)` returned immediately and the
//! caller's loop spun at t=0 (found running a public UVM AVIP). Free tasks
//! (`t()`) and class methods (`obj.m()`) already inlined into the
//! suspend-aware runner; dotted task enables did not. In a class body the
//! call parses as MemberAccess on the property rather than a flattened
//! Ident, so both forms must reach the hierarchical inline stage.
//!
//! Pins (all outputs reference-verified line-for-line):
//! 1. module task + interface task (internal-signal wait and port wait)
//! 2. virtual-interface property call from a class method, with the method
//!    itself reached via `obj.m()` — the full monitor-proxy shape.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_hiertask_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb_top", path.to_str().unwrap(), "--no-cache"])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    text
}

#[test]
fn dotted_module_and_interface_task_calls_suspend() {
    let text = run(
        "instpath",
        r#"module mod_helper (input logic clk);
  task automatic sample_one(output int t_out);
    @(negedge clk);
    t_out = $time;
  endtask
endmodule

interface if_internal (input logic clk);
  logic mirror;
  assign mirror = clk;
  task automatic sample_mirror(output int t_out);
    @(negedge mirror);        // internal interface signal, not the port
    t_out = $time;
  endtask
  task automatic sample_port(output int t_out);
    @(negedge clk);           // the interface PORT
    t_out = $time;
  endtask
endinterface

module tb_top;
  logic tb_c = 0;
  always #5 tb_c = ~tb_c;
  mod_helper u_m (tb_c);
  if_internal u_i (tb_c);
  int t1, t2, t3;
  initial begin
    #7;
    u_m.sample_one(t1);       // module task, port wait
    u_i.sample_mirror(t2);    // interface task, internal-signal wait
    u_i.sample_port(t3);      // interface task, port wait
    $display("mod_port=%0d if_internal=%0d if_port=%0d", t1, t2, t3);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("mod_port=10 if_internal=20 if_port=30"),
        "dotted task calls did not suspend on their event controls:\n{text}"
    );
}

#[test]
fn class_virtual_iface_task_call_suspends() {
    let text = run(
        "classvif",
        r#"interface mon_if (input logic clk);
  int samples = 0;
  task automatic sample_one(output int t_out);
    @(negedge clk);
    samples++;
    t_out = $time;
  endtask
endinterface

class proxy;
  virtual mon_if vif;
  int times[$];
  task run();
    int t;
    repeat (3) begin
      vif.sample_one(t);
      times.push_back(t);
    end
  endtask
endclass

module tb_top;
  logic clk = 0;
  always #5 clk = ~clk;
  mon_if u_if (clk);
  proxy p;
  initial begin
    p = new();
    p.vif = u_if;
    #7;
    p.run();
    if (p.times.size() == 3 && p.times[0] == 10 && p.times[1] == 20 && p.times[2] == 30)
      $display("VIF_WAIT_PASS");
    else
      $display("VIF_WAIT_FAIL %p", p.times);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("VIF_WAIT_PASS"),
        "class virtual-interface task call did not suspend:\n{text}"
    );
}
