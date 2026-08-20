//! §23.8 / §23.10.1 bind-family resolution: a module inserted with `bind`
//! resolves names through the bind TARGET's scope chain. Three shapes that
//! used to fail together:
//!  - a class declared in a bound module, whose object is constructed there
//!    but whose method is CALLED from the top level (via a handle in an
//!    associative array): hierarchical names in the method body must resolve
//!    from the object's creation scope, `%p` on an upward-referenced unpacked
//!    struct printed 0 for every instance but the first, `%0p` printed the
//!    non-compact form, and `%m` printed the caller's scope instead of
//!    creation scope + class + method;
//!  - `always @*` in a bound module reading the host through the host's
//!    MODULE name (`host_mod.clk`): the read resolved to no dependency edge,
//!    so the block fired once at time 0 and never again;
//!  - `$strobe` with `%m` from that block: the postponed-region drain
//!    formatted with whatever scope the last process left, naming the top
//!    module instead of the bound instance.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_bind_upref_{}_{}", tag, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("t.sv");
    std::fs::write(&f, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "tb_top", "--max-time", "100000"])
        .arg(&f)
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn bound_class_method_resolves_from_creation_scope() {
    // Two host instances at different depths; the checker class lives in a
    // bound module and its task is invoked from tb_top's initial through
    // handles registered in an assoc array. Every instance must render ITS
    // OWN host's payload.
    let src = r#"
package chk_pkg;
  virtual class chk_base;
    pure virtual task run_checks();
  endclass
  chk_base registry[string];
endpackage

module host_mod(input logic clk);
  typedef struct { int id; bit active; } pay_t;
  pay_t payload = '{id: 777, active: 1'b1};
endmodule

module host_chk_binder;
  import chk_pkg::*;
  class host_proxy extends chk_base;
    task run_checks();
      $display("MPATH %s", $sformatf("%m"));
      $display("PVAL %p", host_mod.payload);
      $display("P0VAL %0p", host_mod.payload);
    endtask
  endclass
  host_proxy p0;
  initial begin
    p0 = new();
    registry[$sformatf("%m")] = p0;
  end
endmodule

bind host_mod host_chk_binder u_chk();

module mid_wrap(input logic clk);
  host_mod u_deep(.clk(clk));
endmodule

module tb_top;
  import chk_pkg::*;
  logic clk = 0;
  host_mod u_shallow(.clk(clk));
  mid_wrap u_mid(.clk(clk));
  initial begin
    #10;
    foreach (registry[k]) registry[k].run_checks();
    $finish;
  end
endmodule
"#;
    let text = run(src, "class");
    let pvals: Vec<&str> = text.lines().filter(|l| l.starts_with("PVAL ")).collect();
    assert_eq!(pvals.len(), 2, "both proxies must run:\n{}", text);
    for l in &pvals {
        assert_eq!(*l, "PVAL '{id:777, active:1}", "%p upward struct:\n{}", text);
    }
    let p0vals: Vec<&str> = text.lines().filter(|l| l.starts_with("P0VAL ")).collect();
    for l in &p0vals {
        assert_eq!(*l, "P0VAL 777 1", "%0p compact form:\n{}", text);
    }
    // %m = creation scope + class + method, one per instance.
    let mut mpaths: Vec<&str> = text.lines().filter(|l| l.starts_with("MPATH ")).collect();
    mpaths.sort();
    assert_eq!(
        mpaths,
        vec![
            "MPATH tb_top.u_mid.u_deep.u_chk.host_proxy.run_checks",
            "MPATH tb_top.u_shallow.u_chk.host_proxy.run_checks",
        ],
        "%m in class method:\n{}",
        text
    );
}

#[test]
fn bound_always_star_refires_on_host_signal() {
    let src = r#"
module host_mod(input logic clk, input logic en);
endmodule

module host_watch;
  always @* begin
    $strobe("TICK %0d %m clk=%b en=%b", $time, host_mod.clk, host_mod.en);
  end
endmodule

bind host_mod host_watch u_watch();

module tb_top;
  logic clk = 0, en = 0;
  host_mod u_host(.clk(clk), .en(en));
  always #5 clk = ~clk;
  initial begin
    #7 en = 1;
    #21 $finish;
  end
endmodule
"#;
    let text = run(src, "star");
    let ticks: Vec<&str> = text.lines().filter(|l| l.starts_with("TICK ")).collect();
    // clk toggles at 5/10/15/20/25 and en at 7 — the block must re-fire well
    // past time 0 (it used to fire exactly once).
    assert!(
        ticks.len() >= 5,
        "@* in bound module must re-fire on host signal changes, got {:?}:\n{}",
        ticks,
        text
    );
    assert!(
        ticks.iter().any(|l| l.contains(" 15 ") || l.contains(" 20 ")),
        "expected strobes at later ticks:\n{}",
        text
    );
    // %m from $strobe names the bound instance, not the top module.
    for l in &ticks {
        assert!(
            l.contains("tb_top.u_host.u_watch"),
            "$strobe %m must name the bound instance: {}\n{}",
            l,
            text
        );
    }
    assert!(
        ticks.iter().any(|l| l.ends_with("en=1")),
        "en change must reach the strobe:\n{}",
        text
    );
}

#[test]
fn bound_block_upward_write_after_upward_read() {
    // The resolve hint is a ratchet: resolving `dut_top.mode` re-points it
    // at `u_dut`, and the NEXT upward reference in the same block (the NBA
    // target `host_mod.e1`) used to start its walk there and miss — the
    // write landed on a phantom flat name and the host signal stayed x.
    let src = r#"
module side_mod;
  logic [3:0] side_val = 4'hC;
endmodule
module host_mod(input logic clk);
  logic [7:0] level = 8'h20;
  logic [7:0] e1, e2, e3;
endmodule
module host_probe;
  always @(posedge host_mod.clk) begin
    host_mod.e1 <= host_mod.level + dut_top.mode;
    host_mod.e2 <= host_mod.level + dut_top.u_side.side_val;
    host_mod.e3 <= host_mod.level + dut_top.mode + dut_top.u_side.side_val;
  end
endmodule
bind host_mod host_probe u_probe();
module dut_top(input logic clk);
  logic [7:0] mode = 8'h03;
  side_mod u_side();
  host_mod u_h(.clk(clk));
endmodule
module tb_top;
  logic clk = 0;
  dut_top u_dut(.clk(clk));
  always #5 clk = ~clk;
  initial begin
    #12;
    $display("WR e1=%h e2=%h e3=%h", u_dut.u_h.e1, u_dut.u_h.e2, u_dut.u_h.e3);
    $finish;
  end
endmodule
"#;
    let text = run(src, "wr");
    let line = text
        .lines()
        .find(|l| l.starts_with("WR "))
        .unwrap_or_else(|| panic!("no WR line:\n{}", text));
    assert_eq!(line, "WR e1=23 e2=2c e3=2f", "reference values:\n{}", text);
}
