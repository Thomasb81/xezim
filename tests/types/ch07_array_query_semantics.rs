//! IEEE 1800-2023 §7.11 / §20.7 / §20.7.1 array querying semantics:
//! out-of-range / 0 / 'x / negative dimensions, `[k:k]` `$increment`,
//! associative arrays, scalars / `real` / `string` / unpacked structs / `[0:0]`
//! / indexed sub-arrays, unpacked slices, `parameter type`, `type()` operator,
//! mixed fixed+dynamic+fixed multi-D arrays, `O(1)` large struct member
//! queries, and bytecode constant folding.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulation should succeed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_has(lines: &[String], expected: &str) {
    assert!(
        lines.iter().any(|l| l == expected),
        "missing expected line {expected:?} in output:\n{}",
        lines.join("\n")
    );
}

#[test]
fn array_query_bounds_oor_zero_x_single_elem_and_scalars() {
    let src = r#"
module top #(parameter type T = logic [3:0][7:0]);
  int a03[0:3];
  int d[];
  int m[2][3];
  int a55[5:5];
  logic [3:3] v33;
  logic logic_scalar;
  real r_var;
  string s_var;
  typedef struct { int x; int y; } ustruct_t;
  ustruct_t us_var;
  logic [0:0] b00[4];
  int m3d[2][3][4];
  int a05[0:5];
  int a50[5:0];
  T tx;

  localparam integer LP_INT_2 = $left(int, 2);
  localparam integer LP_A03_99 = $left(a03, 99);
  localparam integer LP_A03_0 = $left(a03, 0);
  localparam integer LP_A03_X = $left(a03, 32'bx);
  localparam integer LP_TX_D = $dimensions(tx);
  localparam integer LP_TX_L2 = $left(tx, 2);
  localparam integer LP_TYPE_SZ = $size(type(a05));
  localparam integer LP_TYPE_VEC_L = $left(type(logic [3:0]));

  initial begin
    d = new[2];
    s_var = "hello";
    $display("LP:%b%b%b%b:%0d:%0d:%0d:%0d",
      LP_INT_2 === 32'bx, LP_A03_99 === 32'bx,
      LP_A03_0 === 32'bx, LP_A03_X === 32'bx,
      LP_TX_D, LP_TX_L2, LP_TYPE_SZ, LP_TYPE_VEC_L);
    $display("OOR:%b%b%b%b%b%b",
      $left(a03, 0) === 32'bx,
      $left(a03, 99) === 32'bx,
      $left(a03, 32'bx) === 32'bx,
      $size(d, 0) === 32'bx,
      $left(m[0], 0) === 32'bx,
      $left(a03, -1) === 32'bx);
    $display("INC_EQ:%0d:%0d", $increment(a55), $increment(v33));
    $display("SCALARS:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $dimensions(logic_scalar), $dimensions(r_var), $dimensions(us_var),
      $dimensions(s_var), $dimensions(b00),
      $unpacked_dimensions(m3d[0]), $dimensions(m3d[0]));
    $display("STR_Q:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $unpacked_dimensions(s_var), $left(s_var), $right(s_var),
      $low(s_var), $high(s_var), $increment(s_var), $size(s_var));
    $display("ZERO_Q:%b%b%b",
      $left(logic_scalar) === 32'bx, $left(r_var) === 32'bx, $left(us_var) === 32'bx);
    $display("B00:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $unpacked_dimensions(b00), $left(b00, 1), $right(b00, 1),
      $left(b00, 2), $right(b00, 2), $increment(b00, 2), $size(b00, 2));
    $display("SLICE_ASC:%0d:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $left(a05[2:4]), $right(a05[2:4]), $low(a05[2:4]), $high(a05[2:4]),
      $increment(a05[2:4]), $size(a05[2:4]),
      $dimensions(a05[2:4]), $unpacked_dimensions(a05[2:4]));
    $display("SLICE_DESC:%0d:%0d:%0d:%0d:%0d:%0d",
      $left(a50[4:2]), $right(a50[4:2]), $low(a50[4:2]), $high(a50[4:2]),
      $increment(a50[4:2]), $size(a50[4:2]));
    $display("TYPE_OP:%0d:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
      $size(type(a05)), $left(type(a05)), $right(type(a05)),
      $dimensions(type(a05)), $unpacked_dimensions(type(a05)),
      $size(type(logic [3:0])), $left(type(logic [3:0])),
      type(a05) == type(a05));
    $display("TPARAM:%0d:%0d:%0d", $dimensions(tx), $left(tx, 1), $left(tx, 2));
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_has(&lines, "LP:1111:2:7:6:3");
    assert_has(&lines, "OOR:111111");
    assert_has(&lines, "INC_EQ:1:1");
    assert_has(&lines, "SCALARS:0:0:0:1:2:2:3");
    assert_has(&lines, "STR_Q:0:0:4:0:4:-1:5");
    assert_has(&lines, "ZERO_Q:111");
    assert_has(&lines, "B00:1:0:3:0:0:1:1");
    assert_has(&lines, "SLICE_ASC:2:4:2:4:-1:3:2:1");
    assert_has(&lines, "SLICE_DESC:4:2:2:4:1:3");
    assert_has(&lines, "TYPE_OP:6:0:5:2:1:4:3:1");
    assert_has(&lines, "TPARAM:2:3:7");
}

#[test]
fn array_query_associative_and_mixed_multidim() {
    let src = r#"
module top;
  int aa_int[int];
  int aa_byte[byte];
  int aa_u8[bit [7:0]];
  int aa_wild[*];
  int aa_str[string];
  int md[3][][5];

  initial begin
    $display("AA_INT_E:%0d:%0d:%0d:%0d:%b:%b:%0d:%0d:%0d:%b",
      $dimensions(aa_int), $unpacked_dimensions(aa_int),
      $left(aa_int), $right(aa_int),
      $low(aa_int) === 32'bx, $high(aa_int) === 32'bx,
      $increment(aa_int), $size(aa_int),
      $size(aa_int, 2), $size(aa_int, 3) === 32'bx);
    aa_int[10] = 1;
    aa_int[-5] = 2;
    aa_int[42] = 3;
    $display("AA_INT_P:%0d:%0d:%0d:%0d:%0d:%0d",
      $left(aa_int), $right(aa_int), $low(aa_int), $high(aa_int),
      $increment(aa_int), $size(aa_int));

    $display("AA_U8_E:%0d:%0d:%0d:%0d:%b:%b:%0d:%0d",
      $dimensions(aa_u8), $unpacked_dimensions(aa_u8),
      $left(aa_u8), $right(aa_u8),
      $low(aa_u8) === 32'bx, $high(aa_u8) === 32'bx,
      $increment(aa_u8), $size(aa_u8));
    aa_u8[7] = 1;
    aa_u8[200] = 2;
    $display("AA_U8_P:%0d:%0d:%0d:%0d:%0d:%0d",
      $left(aa_u8), $right(aa_u8), $low(aa_u8), $high(aa_u8),
      $increment(aa_u8), $size(aa_u8));

    aa_byte[-10] = 1;
    aa_byte[20] = 2;
    $display("AA_BYTE:%0d:%0d:%0d:%0d:%0d:%0d",
      $left(aa_byte), $right(aa_byte), $low(aa_byte), $high(aa_byte),
      $increment(aa_byte), $size(aa_byte));

    aa_wild[3] = 1;
    aa_wild[99] = 2;
    $display("AA_WILD:%0d:%0d:%0d:%0d:%0d:%0d",
      $dimensions(aa_wild), $unpacked_dimensions(aa_wild),
      $low(aa_wild), $high(aa_wild), $increment(aa_wild), $size(aa_wild));

    $display("AA_STR:%0d:%0d:%0d",
      $dimensions(aa_str), $unpacked_dimensions(aa_str), $size(aa_str));

    md[2] = new[4];
    md[2][2][0] = 220;
    $display("MD:%0d:%0d:%0d:%0d:%0d:%b:%0d:%0d:%0d:%0d",
      $dimensions(md), $unpacked_dimensions(md),
      $size(md, 1), $size(md, 3), $size(md, 4),
      $size(md, 5) === 32'bx,
      $size(md[2], 1), $size(md[1], 1), $unpacked_dimensions(md[2]), $size(md[2], 2));
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_has(&lines, "AA_INT_E:2:1:0:2147483647:1:1:-1:0:32:1");
    assert_has(&lines, "AA_INT_P:0:2147483647:-5:42:-1:3");
    assert_has(&lines, "AA_U8_E:2:1:0:255:1:1:-1:0");
    assert_has(&lines, "AA_U8_P:0:255:7:200:-1:2");
    assert_has(&lines, "AA_BYTE:0:127:-10:20:-1:2");
    assert_has(&lines, "AA_WILD:2:1:3:99:-1:2");
    assert_has(&lines, "AA_STR:2:1:0");
    assert_has(&lines, "MD:4:3:3:5:32:1:4:0:2:5");
}

#[test]
fn array_query_struct_member_o1_and_bytecode_folding() {
    let src = r#"
module top;
  typedef struct {
    int big[100000];
  } big_s_t;
  big_s_t bs;

  logic [3:0][7:0] pk [2:5];
  logic clk = 0;
  integer acc = 0;
  integer oor_x = 0;
  integer scalar_dim = -1;

  always #5 clk = ~clk;

  always @(posedge clk) begin
    acc <= acc + $left(pk) + $increment(pk, 1) + $increment(pk, 2) + $size(pk, 3);
    oor_x <= ($left(pk, 99) === 32'bx);
    scalar_dim <= $dimensions(bs);
  end

  initial begin
    $display("BIG:%0d:%0d:%0d", $left(bs.big), $right(bs.big), $size(bs.big));
    #35;
    $display("BC:%0d:%0d:%0d", acc, oor_x, scalar_dim);
    $finish;
  end
endmodule
"#;
    let lines = sim_lines(src);
    // 2 (left pk) + (-1) (inc dim1) + 1 (inc dim2) + 8 (size dim3) = 10 per posedge
    // Three posedges at t=5, 15, 25 -> acc = 30
    assert_has(&lines, "BIG:0:99999:100000");
    assert_has(&lines, "BC:30:1:0");
}
