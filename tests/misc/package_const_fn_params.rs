//! Package parameters whose initializers CALL constant functions —
//! reference-validated (customer X-storm root cause). Three failure modes
//! covered: (1) the package hoist evaluated fn-call inits to 0 before the
//! package's functions were registered, so every `p::PARAM` dimension in a
//! non-importing module resolved against 0; (2) a later re-eval arm
//! clobbered the healed value back to 0; (3) struct-member LAYOUTS of a
//! scoped `pkg::T` resolved member dims with the USING module's params, so
//! member writes/reads through the layout silently vanished.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const PKG: &str = r#"
package cfg;
  function automatic integer LOG2(input integer v);
    integer r;
    begin r = 0; while (v > 1) begin v = v / 2; r = r + 1; end LOG2 = r; end
  endfunction
  parameter integer DEPTH = 64;
  parameter integer LOG_DEPTH = LOG2(DEPTH);       // 6, arg is a sibling param
  parameter integer LOG_LIT   = LOG2(64);          // 6, literal arg
  parameter integer LOG_M6    = LOG2(DEPTH) - 6;   // 0 (the "-6 clamp" shape)
  typedef struct packed {
    logic [LOG_DEPTH-1:0]   addr;                  // 6
    logic [2*LOG_DEPTH-1:0] tag;                   // 12
    logic                   vld;                   // 1
  } req_t;                                         // 19 bits
endpackage
"#;

#[test]
fn scoped_fn_param_dimension_without_import() {
    // No import anywhere: the dims must still see LOG_DEPTH=6, not 0.
    let src = format!(
        "{PKG}
module tb;
  logic [cfg::LOG_DEPTH-1:0] a;
  logic [cfg::LOG_LIT-1:0]   b;
  initial $display(\"T|%0d %0d m6=%0d\", $bits(a), $bits(b), cfg::LOG_M6);
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|6 6 m6=0"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn scoped_struct_member_layout_without_import() {
    // Reference: r=54001 (addr at [18:13], vld at [0]). The layout bug read
    // addr back as 0 and dropped the write entirely.
    let src = format!(
        "{PKG}
module tb;
  cfg::req_t r;
  initial begin
    r = '0; r.addr = 6'h2a; r.vld = 1'b1;
    #1 $display(\"T|r=%h addr=%h vld=%b\", r, r.addr, r.vld);
  end
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|r=54001 addr=2a vld=1"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn scoped_struct_port_crosses_module_boundary() {
    // The child's port type is the scoped struct; before the fix the port
    // resolved to 5 bits (width mismatch warning) and the value truncated.
    let src = format!(
        "{PKG}
module child(input cfg::req_t req, output logic [31:0] echo);
  assign echo = {{13'h0, req}};
endmodule
module tb;
  cfg::req_t r;
  logic [31:0] e;
  child u(.req(r), .echo(e));
  initial begin
    r = '0; r.addr = 6'h2a; r.vld = 1'b1;
    #1 $display(\"T|echo=%h\", e);
  end
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|echo=00054001"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn same_name_params_stay_per_package_and_module_shadow_wins() {
    // Overlaying the owning package's params for a layout walk must not leak
    // across packages or override a module-local param OUTSIDE the walk.
    let src = r#"
package pa;
  function automatic integer L2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end L2=r; end
  endfunction
  parameter integer W = L2(16);                              // 4
  typedef struct packed { logic [W-1:0] f; logic v; } sa_t;  // 5
endpackage
package pb;
  parameter integer W = 9;
  typedef struct packed { logic [W-1:0] f; logic v; } sb_t;  // 10
endpackage
module tb;
  parameter integer W = 2;
  pa::sa_t a;
  pb::sb_t b;
  logic [W-1:0] m;                                           // module W=2
  initial begin
    a = '0; a.f = 4'hf; b = '0; b.f = 9'h155;
    #1 $display("T|%0d %0d %0d a=%h b=%h", $bits(a), $bits(b), $bits(m), a, b);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|5 10 2 a=1e b=2aa"),
        "got {:?}",
        msgs(&sim)
    );
}
