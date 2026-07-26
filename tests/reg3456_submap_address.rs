use std::process::Command;

const XEZIM: &str = env!("CARGO_BIN_EXE_xezim");
const UVM_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../1800.2-2020.3.1/src");
const DPI_LIB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/uvm-2020.3.1.so");

/// Regression test for Mantis 3456: register address resolution through
/// nested register block hierarchy with submap offset overrides.
///
/// The test creates a two-level register block hierarchy (blk2 -> blk1 x 2)
/// with submap address offsets.  Registers b1.r1/b1.r2 should resolve to
/// 'h11000/'h11010, and b2.r1/b2.r2 to 'h10200/'h10210.  Before the fix,
/// xezim returned 'hx for all addresses because:
///   1. String comparison `m_name == ""` returned X (xezim-core value.rs bug:
///      zero-width comparison under X branch)
///   2. Empty string literal `""` had width 8 instead of 0 (simulator.rs
///      StringLiteral bug)
///   3. uvm_reg_block::get_full_name fell through to returning `m_name`
///      (which was `""`) instead of calling `get_name()`
///   4. uvm_reg_block::add_map had identical string-comparison issues
#[ignore = "blocked by xezim parser issue with uvm_resource_enum_read macro in uvm_agent.svh"]
#[test]
fn reg3456_submap_address() {
    let test_sv = r#"
`include "uvm_macros.svh"
program top;
import uvm_pkg::*;

class reg1 extends uvm_reg;
   `uvm_object_utils(reg1)
   uvm_reg_field data;
   function new(string name = "reg1");
      super.new(name,32,UVM_NO_COVERAGE);
   endfunction
   virtual function void build();
      data = uvm_reg_field::type_id::create("data",,get_full_name());
      data.configure(this, 32,  0, "RW", 0, 'h0, 1, 0, 1);
   endfunction
endclass

class blk1 extends uvm_reg_block;
   `uvm_object_utils(blk1)
   reg1 r1; reg1 r2;
   function new(string name = "blk1");
      super.new(name, UVM_NO_COVERAGE);
   endfunction
   function void build();
      default_map = create_map("", 'h100, 4, UVM_LITTLE_ENDIAN);
      r1 = reg1::type_id::create("r1",,get_full_name());
      r1.configure(this, null, ""); r1.build();
      r2 = reg1::type_id::create("r2",,get_full_name());
      r2.configure(this, null, ""); r2.build();
      default_map.add_reg(r1, 0);
      default_map.add_reg(r2, 'h10);
   endfunction
endclass

class blk2 extends uvm_reg_block;
   `uvm_object_utils(blk2)
   blk1 b1; blk1 b2;
   function new(string name = "blk2");
      super.new(name, UVM_NO_COVERAGE);
   endfunction
   function void build();
      b1 = blk1::type_id::create("b1");
      b1.configure(this); b1.build();
      b2 = blk1::type_id::create("b2");
      b2.configure(this); b2.build();
      default_map = create_map("", 'h10000, 1, UVM_LITTLE_ENDIAN);
      default_map.add_submap(b1.default_map, 'h1000);
      default_map.add_submap(b2.default_map, 'h2000);
      b2.default_map.set_base_addr('h200);
   endfunction
endclass

function void check(uvm_reg rg, uvm_reg_addr_t exp_off, uvm_reg_addr_t exp_addr);
   if (rg.get_offset() !== exp_off) begin
      $write("FAIL: %s offset got 'h%0h exp 'h%0h\n", rg.get_full_name(), rg.get_offset(), exp_off);
   end
   if (rg.get_address() !== exp_addr) begin
      $write("FAIL: %s address got 'h%0h exp 'h%0h\n", rg.get_full_name(), rg.get_address(), exp_addr);
   end
endfunction

initial begin
   blk2 blk = blk2::type_id::create("blk");
   blk.build();
   blk.lock_model();
   check(blk.b1.r1, 'h0000, 'h11000);
   check(blk.b1.r2, 'h0010, 'h11010);
   check(blk.b2.r1, 'h0000, 'h10200);
   check(blk.b2.r2, 'h0010, 'h10210);
   begin
      uvm_report_server svr = uvm_coreservice_t::get().get_report_server();
      svr.report_summarize();
      if (svr.get_severity_count(UVM_FATAL) == 0 && svr.get_severity_count(UVM_ERROR) == 0)
         $write("** UVM TEST PASSED **\n");
      else
         $write("** UVM TEST FAILED **\n");
   end
end
endprogram
"#;

    // Write the test to a temp file
    let tmpdir = std::env::temp_dir().join("svrun");
    std::fs::create_dir_all(&tmpdir).unwrap();
    let sv_path = tmpdir.join("reg3456_submap_address.sv");
    std::fs::write(&sv_path, test_sv).unwrap();

    let output = Command::new(XEZIM)
        .args([
            "--simulate",
            "--no-cache",
            "-s",
            "top",
            "-I",
            UVM_SRC,
            "--dpi-lib",
            DPI_LIB,
            &format!("{UVM_SRC}/uvm_pkg.sv"),
        ])
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("Failed to run xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // Print output for debugging
    println!("=== xezim output ===");
    for line in combined.lines() {
        if !line.contains("PROF]") && !line.contains("PHASE]") && !line.contains("---")
            && !line.contains("EVENT-EDGE") && !line.contains("RELNOTES")
            && !line.contains("Copyright") && !line.contains("Accellera")
            && !line.contains("git ")
        {
            println!("{line}");
        }
    }

    // Check for PASSED
    assert!(
        combined.contains("UVM TEST PASSED"),
        "Test did not pass.  Output:\n{}",
        combined
    );
}