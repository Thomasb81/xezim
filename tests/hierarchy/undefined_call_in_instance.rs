//! A call to a function/task that no scope in the design declares must be
//! an elaboration error, including inside a sub-instance: those bodies are
//! inlined after the per-module identifier check ran, so an RTL assertion
//! helper bound to a stale header that lacked it silently never executed.
//! Also covers the `include shadowing warning that made the field case hard
//! to see: two search dirs with a same-named, different header.
use std::path::PathBuf;
use std::process::Command;

const CHILD_CALLS_UNDEFINED: &str = r#"
package common_types_pkg;
   typedef logic [0:0] bit_t;
endpackage
module glue(input logic clk, input logic rst_l, input logic cond);
   import common_types_pkg::*;
   always @(posedge clk) begin
      chk_always((rst_l !== 1), |(cond), $sformatf("in %m: cond violated"))
      ;
   end
endmodule
module testbench;
   logic clk = 0, rst_l = 0, cond = 0;
   always #5 clk = ~clk;
   initial begin #10 rst_l = 1; #20 $finish; end
   glue u_glue(.clk(clk), .rst_l(rst_l), .cond(cond));
endmodule
"#;

/// Every legal call shape a sub-instance can make: package function
/// imported only in the child, $unit function, static and instance class
/// methods, interface method, forward-referenced local function, task,
/// generate-block function, typedef and builtin casts, randomize.
const CHILD_LEGAL_CALLS: &str = r#"
function automatic int unit_fn(int a); return a + 1; endfunction
package pk; function automatic int pk_fn(int a); return a * 2; endfunction endpackage
class Cls; static function int sm(int a); return a + 100; endfunction function int im(int a); return a + 7; endfunction endclass
interface ifc; logic [7:0] v; function int get(); return v; endfunction endinterface
typedef logic [7:0] byte_t;
module child(input logic clk, ifc i);
  import pk::*;
  int acc = 0; Cls c = new;
  function automatic int later_defined(int a); return local_helper(a); endfunction
  function automatic int local_helper(int a); return a - 1; endfunction
  task automatic bump(int k); acc += k; endtask
  generate if (1) begin : g
    function automatic int gfn(int a); return a << 1; endfunction
    always @(posedge clk) acc <= acc + gfn(0);
  end endgenerate
  always @(posedge clk) begin
    acc <= acc + pk_fn(1) + unit_fn(1) + Cls::sm(1) + c.im(1) + i.get() + later_defined(3) + byte_t'(acc) + int'(1.5);
    bump(1);
    void'(c.randomize());
  end
  final $display("SMOKE acc=%0d", acc);
endmodule
module top;
  logic clk = 0; ifc i(); assign i.v = 8'd5;
  child u(.clk(clk), .i(i));
  initial begin repeat (2) #5 clk = ~clk; #1 $finish; end
endmodule
"#;

const HEADER_OLD: &str = "package pkg_h; typedef logic t; endpackage\n";
const HEADER_NEW: &str =
    "package pkg_h; typedef logic t; function void helper(input c); endfunction endpackage\n";
const USES_HEADER: &str = r#"
`include "shared.h"
module top;
  import pkg_h::*;
  initial begin helper(1); $finish; end
endmodule
"#;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("undefined_call").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_xezim")).args(args).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn undefined_call_inside_a_sub_instance_is_an_error() {
    let d = dir("undef");
    let src = d.join("tb.sv");
    std::fs::write(&src, CHILD_CALLS_UNDEFINED).unwrap();
    let (ok, text) = run(&["--simulate", "-s", "testbench", src.to_str().unwrap()]);
    assert!(!ok, "undefined call was accepted:\n{text}");
    assert!(
        text.contains("Undeclared identifier 'chk_always'"),
        "no diagnostic naming the undefined call:\n{text}"
    );
}

#[test]
fn every_legal_call_shape_in_a_sub_instance_still_elaborates() {
    let d = dir("legal");
    let src = d.join("top.sv");
    std::fs::write(&src, CHILD_LEGAL_CALLS).unwrap();
    let (ok, text) = run(&["--simulate", "-s", "top", src.to_str().unwrap()]);
    assert!(ok, "legal calls rejected:\n{text}");
    assert!(text.contains("SMOKE acc=122"), "wrong result:\n{text}");
}

#[test]
fn shadowed_include_copy_is_reported_and_missing_helper_is_an_error() {
    let d = dir("shadow");
    let old = d.join("old");
    let new = d.join("new");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(old.join("shared.h"), HEADER_OLD).unwrap();
    std::fs::write(new.join("shared.h"), HEADER_NEW).unwrap();
    let src = d.join("top.sv");
    std::fs::write(&src, USES_HEADER).unwrap();
    let inc_old = format!("+incdir+{}", old.display());
    let inc_new = format!("+incdir+{}", new.display());
    let (ok, text) =
        run(&["--simulate", "-s", "top", src.to_str().unwrap(), &inc_old, &inc_new]);
    assert!(!ok, "stale header accepted:\n{text}");
    assert!(text.contains("Undeclared identifier 'helper'"), "no call diagnostic:\n{text}");
    assert!(
        text.contains("[PP] warning: `include \"shared.h\"") && text.contains("is shadowed"),
        "no shadowing warning:\n{text}"
    );
    // Search order reversed: the newer copy wins and the design runs.
    let (ok, text) =
        run(&["--simulate", "-s", "top", src.to_str().unwrap(), &inc_new, &inc_old]);
    assert!(ok, "newer header rejected:\n{text}");
}
