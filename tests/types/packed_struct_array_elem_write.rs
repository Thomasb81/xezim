//! §7.4.2 — writing a packed ELEMENT of a struct member inside a PACKED array
//! of packed structs: `arr[i].field[k] = v`. Reference-validated.
//!
//! `arr[i]` is not a signal of its own here — a packed array of packed structs
//! is one backing vector — so the write must splice at
//! `slot*struct_w + field_off + k*member_elem_w`. The existing member path
//! handled `arr[i].field = v`, but this form's outermost AST node is an Index,
//! so it never reached that code and the write was silently DROPPED.
//!
//! The read path resolved the same expression correctly all along, which is
//! what made the value look like it was never driven rather than never stored:
//! reading back gave 0 with no error anywhere.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package P;
  typedef struct packed {
    logic [1:0][63:0] wdata;
    logic [1:0][7:0]  mask;
    logic [1:0]       amask;
  } t;
endpackage

module tb;
  P::t       s;      // plain struct, for contrast
  P::t [1:0] c1;     // packed array of packed structs

  logic [63:0] r_plain, r_e1w0, r_e1w1, r_e0w0;
  logic [1:0]  r_amask;

  initial begin
    s = '0; c1 = '0;
    s.wdata[0]     = 64'h11;
    c1[1].wdata[0] = 64'h22;    // the form that was dropped
    c1[1].wdata[1] = 64'h33;
    c1[0].wdata[0] = 64'h44;    // a different element must not collide
    c1[0].amask    = 2'b11;     // member with no trailing index still works
    #1;
    r_plain  = s.wdata[0];
    r_e1w0   = c1[1].wdata[0];
    r_e1w1   = c1[1].wdata[1];
    r_e0w0   = c1[0].wdata[0];
    r_amask  = c1[0].amask;
  end
endmodule
"#;

#[test]
fn packed_array_of_struct_member_element_writes_land() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r_plain"), 0x11, "plain struct member element (control)");
    assert_eq!(u(&sim, "r_e1w0"), 0x22, "c1[1].wdata[0] — this write used to be dropped");
    assert_eq!(u(&sim, "r_e1w1"), 0x33, "c1[1].wdata[1] — second element of the same member");
    assert_eq!(
        u(&sim, "r_e0w0"),
        0x44,
        "c1[0].wdata[0] — a different array element must not alias c1[1]"
    );
    assert_eq!(u(&sim, "r_amask"), 0b11, "member with no trailing index (control)");
}
