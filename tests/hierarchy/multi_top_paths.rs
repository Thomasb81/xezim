//! Several top-level modules (§23.3.3) elaborate under one synthetic root,
//! but that root is not part of the design: each top prints, resolves and
//! dumps exactly as it would standing alone, whether the tops are found
//! automatically or named with `-s`. The expected lines were cross-checked
//! against the reference simulator, which treats every top as a root.

use std::process::Command;

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_multi_top_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run xezim on `src` in a fresh directory; returns stdout + stderr.
fn run(tag: &str, src: &str, args: &[&str]) -> (String, std::path::PathBuf) {
    let d = scratch(tag);
    std::fs::write(d.join("t.sv"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&d)
        .arg("t.sv")
        .args(args)
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "xezim {:?} failed:\n{}", args, text);
    (text, d)
}

const DESIGN: &str = r#"
module tb #(parameter int W = 4) ();
  logic [W-1:0] x = 5;
  sub #(.W(6)) u();
  function void f(); $display("fn %m"); endfunction
  initial begin
    $display("%m W=%0d bits=%0d x=%0d", W, $bits(logic [W-1:0]), $root.tb.x);
    $display("sformatf %s", $sformatf("%m"));
    f();
    begin : blk $display("blk %m"); end
    $printtimescale;
    #1 $display("t=%0t in %m sib=%0d", $time, other.y);
  end
endmodule
module sub #(parameter int W = 3);
  initial $display("%m W=%0d bits=%0d", W, $bits(logic [W-1:0]));
endmodule
module other;
  logic [7:0] y = 9;
  initial $display("%m tbx=%0d", tb.x);
endmodule
"#;

fn check_design(out: &str) {
    for want in [
        "tb.u W=6 bits=6",
        "tb W=4 bits=4 x=5",
        "sformatf tb\n",
        "fn tb.f",
        "blk tb.blk",
        "Time scale of (tb) is",
        "other tbx=5",
        "t=1 in tb sib=9",
    ] {
        assert!(out.contains(want), "missing `{}`:\n{}", want, out);
    }
    assert!(
        !out.contains("__xezim") && !out.contains("__xz"),
        "wrapper name leaked:\n{}",
        out
    );
}

#[test]
fn auto_detected_tops_print_their_own_paths() {
    check_design(&run("auto", DESIGN, &[]).0);
}

#[test]
fn named_tops_print_their_own_paths() {
    check_design(&run("named", DESIGN, &["-s", "tb", "-s", "other"]).0);
}

/// A child's `$bits(<type>)` sizes from the child's own parameters even with
/// a single top (the root's `W` was used before).
#[test]
fn type_operand_bits_use_the_instance_parameters() {
    let src = r#"
module tb #(parameter int W = 4) ();
  sub #(.W(6)) u();
  initial $display("%m W=%0d bits=%0d", W, $bits(logic [W-1:0]));
endmodule
module sub #(parameter int W = 3);
  initial $display("%m W=%0d bits=%0d", W, $bits(logic [W-1:0]));
endmodule
"#;
    let (out, _) = run("bits", src, &[]);
    assert!(out.contains("tb.u W=6 bits=6"), "{}", out);
    assert!(out.contains("tb W=4 bits=4"), "{}", out);
}

/// Each top is a root scope of the VCD, as it is standing alone.
#[test]
fn vcd_has_one_root_scope_per_top() {
    let src = r#"
module tb;
  logic clk = 0;
  sub u();
  initial begin $dumpfile("w.vcd"); $dumpvars; #5 clk = 1; #5 $finish; end
endmodule
module sub; logic [1:0] q = 1; endmodule
module other; logic [7:0] y = 9; endmodule
"#;
    for args in [&["--wave"][..], &["--wave", "-s", "tb", "-s", "other"][..]] {
        let (_, d) = run("vcd", src, args);
        let vcd = std::fs::read_to_string(d.join("w.vcd")).unwrap();
        let header = &vcd[..vcd.find("$enddefinitions").unwrap()];
        let scopes: Vec<&str> = header
            .lines()
            .filter(|l| l.starts_with("$scope") || l.starts_with("$upscope"))
            .collect();
        assert_eq!(
            scopes,
            [
                "$scope module other $end",
                "$upscope $end",
                "$scope module tb $end",
                "$scope module u $end",
                "$upscope $end",
                "$upscope $end",
            ],
            "{:?}:\n{}",
            args,
            header
        );
    }
}
