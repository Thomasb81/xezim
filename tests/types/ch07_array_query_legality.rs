//! IEEE 1800-2023 §20.7 / §20.7.1 array querying legality checks:
//! - `$dimensions` / `$unpacked_dimensions` take only 1 argument;
//! - bound queries on `n > 1` are rejected when dimension `n` is variable-sized;
//! - array querying functions applied directly to dynamically sized type
//!   identifiers are rejected;
//! - `$left`/`$right`/`$low`/`$high`/`$increment` on associative arrays require
//!   an integral index type (while `$size`, `$dimensions`, `$unpacked_dimensions`
//!   remain legal).

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("legal design should compile and simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(sim) => panic!(
            "expected design to be rejected:\n{src}\noutput: {:?}",
            sim.output
        ),
        Err(e) => e,
    }
}

#[test]
fn rejects_illegal_array_queries_and_accepts_valid_counterparts() {
    // Extra argument to $dimensions / $unpacked_dimensions (§20.7)
    let e1 = rejected("module top; int a[4]; initial $display($dimensions(a, 1)); endmodule");
    assert!(e1.contains("argument"), "unexpected error: {e1}");
    let e2 =
        rejected("module top; int a[4]; initial $display($unpacked_dimensions(a, 1)); endmodule");
    assert!(e2.contains("argument"), "unexpected error: {e2}");

    // Query with n > 1 targeting variable-sized dimension (§20.7.1)
    let e3 = rejected("module top; int a[3][]; initial $display($size(a, 2)); endmodule");
    assert!(e3.contains("variable-sized"), "unexpected error: {e3}");
    let e4 = rejected("module top; int a[3][5][]; initial $display($left(a, 3)); endmodule");
    assert!(e4.contains("variable-sized"), "unexpected error: {e4}");

    // Direct query on dynamically sized type identifier (§20.7)
    let e5 = rejected("module top; typedef int d_t[]; initial $display($size(d_t)); endmodule");
    assert!(e5.contains("dynamically sized"), "unexpected error: {e5}");
    let e6 =
        rejected("module top; typedef int q_t[$]; initial $display($dimensions(q_t)); endmodule");
    assert!(e6.contains("dynamically sized"), "unexpected error: {e6}");

    // Bound queries on non-integral associative array dimensions (§20.7)
    for fn_name in ["$left", "$right", "$low", "$high", "$increment"] {
        let src = format!(
            "module top; int aa[string]; initial $display({fn_name}(aa)); endmodule"
        );
        let err = rejected(&src);
        assert!(err.contains("integral index"), "unexpected error for {fn_name}: {err}");
    }

    // Legal counterparts must compile cleanly and match §20.7 / §20.7.1.
    let ok_lines = sim_lines(
        r#"
module top;
  typedef int d_t[];
  d_t d;
  int a[3][][5];
  int aa[string];
  initial begin
    d = new[2];
    a[2] = new[4];
    aa["k"] = 7;
    $display("OK:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $size(d), $size(a, 1), $size(a, 3), $size(a[2], 1),
      $dimensions(aa), $unpacked_dimensions(aa), $size(aa));
  end
endmodule
"#,
    );
    assert_eq!(ok_lines, vec!["OK:2:3:5:4:2:1:1"]);
}
