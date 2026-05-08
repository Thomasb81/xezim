//! Miri-targeted test for the c910 settle/NBA path.
//!
//! Run with:  cargo +nightly miri test --test c910_settle_miri
//!
//! Reproduces the shape of c910's PRF: shared register-file array written
//! by multiple always_ff blocks, read combinationally with sensitivity
//! fanout to several downstream consumers. If any unsafe pointer/aliasing
//! bug exists in the settle hot loop or NBA accumulation, miri's
//! Stacked-Borrows checker should flag it during a short simulation.

use xezim::simulate;

const SETTLE_REPRO: &str = r#"
module tb();
  reg clk = 0;
  reg rst_n = 0;
  always #5 clk = ~clk;

  reg [63:0] prf [0:15];

  reg [3:0]  w0_addr;  reg [63:0] w0_data;  reg w0_en;
  reg [3:0]  w1_addr;  reg [63:0] w1_data;  reg w1_en;
  reg [3:0]  w2_addr;  reg [63:0] w2_data;  reg w2_en;
  reg [3:0]  w3_addr;  reg [63:0] w3_data;  reg w3_en;

  reg [3:0]  r0_addr;  wire [63:0] r0_data = prf[r0_addr];
  reg [3:0]  r1_addr;  wire [63:0] r1_data = prf[r1_addr];

  always @(posedge clk) if (rst_n && w0_en) prf[w0_addr] <= w0_data;
  always @(posedge clk) if (rst_n && w1_en) prf[w1_addr] <= w1_data;
  always @(posedge clk) if (rst_n && w2_en) prf[w2_addr] <= w2_data;
  always @(posedge clk) if (rst_n && w3_en) prf[w3_addr] <= w3_data;

  reg  [63:0] acc;
  wire [64:0] sum_wide = {1'b0, acc} + {1'b0, r0_data};
  reg  [64:0] sum_lat;
  always @(posedge clk) sum_lat <= sum_wide;

  integer i;
  reg [7:0] cycles;
  initial begin
    rst_n = 0;
    cycles = 0;
    acc = 64'hDEADBEEF_CAFEBABE;
    for (i = 0; i < 16; i = i + 1) prf[i] = {32'h0, i[31:0]};
    w0_en = 0; w1_en = 0; w2_en = 0; w3_en = 0;
    #20;
    rst_n = 1;
  end

  always @(posedge clk) begin
    if (rst_n) begin
      cycles <= cycles + 1;
      w0_addr <= cycles[3:0];
      w1_addr <= cycles[3:0] ^ 4'h4;
      w2_addr <= cycles[3:0] ^ 4'h8;
      w3_addr <= cycles[3:0] ^ 4'hC;
      w0_data <= {32'h0, {24'h0, cycles}};
      w1_data <= {32'h1, {24'h0, cycles}};
      w2_data <= {32'h2, {24'h0, cycles}};
      w3_data <= 64'hFFFFFFFF_00000000 + {32'h0, {24'h0, cycles}};
      w0_en <= 1; w1_en <= 1; w2_en <= 1; w3_en <= 1;
      r0_addr <= cycles[3:0];
      r1_addr <= cycles[3:0] ^ 4'h7;
      acc <= acc + r0_data + r1_data;
      if (cycles == 8'd24) $finish;
    end
  end
endmodule
"#;

#[test]
fn c910_prf_settle_shape() {
    // Run a short simulation. Under miri, this fires the settle hot loop,
    // exec_insns, NBA accumulation, and apply_nba multiple times — enough
    // to catch most pointer/aliasing UB in those paths.
    let sim = simulate(SETTLE_REPRO, 500).expect("simulate failed");
    // Sanity: simulator ran past time 0.
    assert!(sim.time > 0);
}

#[test]
fn wide_arith_carry_not_dropped() {
    // Targeted regression: 0xFFFF_FFFF_FFFF_FFFF + 1 at 65-bit width must
    // produce 0x1_0000_0000_0000_0000, not 0. This was the root cause of
    // c906 cmark "Cannot validate operation" before xezim-core 710a793.
    use xezim::compiler::Value;
    let a = Value::from_u64(u64::MAX, 64);
    let b = Value::from_u64(1, 64);
    let mut a65 = a.resize(65);
    let b65 = b.resize(65);
    a65 = a65.add(&b65);
    // The 65-bit sum has bit 64 set (carry-out).
    let bit64 = a65.get_bit(64);
    assert!(matches!(bit64, xezim_core::value::LogicBit::One),
            "65-bit add lost the carry into bit 64");
    // Low 64 bits should be 0.
    let low = a65.to_u64().unwrap_or(u64::MAX);
    assert_eq!(low, 0, "low 64 bits should be 0 after FFFF...+1");
}
