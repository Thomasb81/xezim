use std::process::Command;

/// Pure-SystemVerilog regression test for Mantis 3456:
/// String comparison with empty strings must return 1, not X.
///
/// Root cause: `Value::is_equal()` returned X when comparing zero-width
/// values through the X-handling branch (no bits to compare, fell through
/// to `return Value::new(1)` which is all-X).  Fix: early-return 1 when
/// both values are zero-width.
///
/// Additionally, `StringLiteral("")` must produce a width-0 value, not
/// width-8, to prevent width-mismatch in `is_equal` from widening the
/// zero-width class property to 8 bits where its X bits cause X return.
#[test]
fn string_compare_empty() {
    let sv = r#"
module test;
   string empty = "";
   string empty2 = "";
   bit eq1 = ("" == "");
   bit eq2 = (empty == "");
   bit eq3 = (empty == empty2);
   bit eq4 = (empty.len() == 0);
   bit eq5 = ("" != "a");

   initial begin
      #1;
      if (eq1 !== 1 || eq2 !== 1 || eq3 !== 1 || eq4 !== 1 || eq5 !== 1) begin
         $write("FAIL: empty string comparison\n");
         $write("  \"\"==\"\": %0d (exp 1)\n", eq1);
         $write("  empty==\"\": %0d (exp 1)\n", eq2);
         $write("  empty==empty2: %0d (exp 1)\n", eq3);
         $write("  empty.len()==0: %0d (exp 1)\n", eq4);
         $write("  \"\"!=\"a\": %0d (exp 1)\n", eq5);
         $fatal(1);
      end
      $write("TAG_PASS\n");
   end
endmodule
"#;

    let tmpdir = std::env::temp_dir().join("svrun");
    std::fs::create_dir_all(&tmpdir).unwrap();
    let sv_path = tmpdir.join("reg3456_pure_sv.sv");
    std::fs::write(&sv_path, sv).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "test"])
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("Failed to run xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if !combined.contains("TAG_PASS") {
        panic!("Test failed.\nOutput:\n{combined}");
    }

    // Also cross-check with reference simulator
    // (QuestaSim must be sourced)
}

/// Test that a string property initialized to "" compares correctly
/// in a class context (mirrors the uvm_reg_block::m_name pattern).
#[test]
fn class_empty_string_property() {
    let sv = r#"
class A;
   string m_name = "";
   function string get_name();
      return m_name;
   endfunction
   function bit is_empty();
      return (m_name == "");
   endfunction
endclass

module test;
   A a = new();
   bit r;

   initial begin
      #1;
      r = a.is_empty();
      if (r !== 1) begin
         $write("FAIL: a.is_empty() = %0d (expected 1)\n", r);
         $fatal(1);
      end
      $write("TAG_PASS\n");
   end
endmodule
"#;

    let tmpdir = std::env::temp_dir().join("svrun");
    std::fs::create_dir_all(&tmpdir).unwrap();
    let sv_path = tmpdir.join("class_empty_string_property.sv");
    std::fs::write(&sv_path, sv).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "test"])
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("Failed to run xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if !combined.contains("TAG_PASS") {
        panic!("Test failed.\nOutput:\n{combined}");
    }
}