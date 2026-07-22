//! §6.20.2 — a task/block-local `localparam`/`parameter` with no explicit type
//! is SELF-DETERMINED from its initializer (32-bit for the usual integer
//! constant), NOT the 1-bit that an implicit type resolves to. Previously such
//! a local localparam truncated to 1 bit and read 0/-1, so `x / QUANTUM` (with
//! `localparam QUANTUM = 24` reading 0) divided by zero and produced x —
//! surfacing to a customer as x-bits in an `8'(...)`-packed register field.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn task_local_localparam_is_full_width() {
    let src = r#"
module t;
  task automatic run();
    localparam Q = 24;          // untyped -> 32-bit, not 1-bit
    localparam SEVEN = 7;
    int bw; logic [31:0] hdr;
    bw = 1200;
    hdr = 32'd0;
    hdr[31:24] = 8'((bw / Q) - 1);   // 8'(49) = 0x31
    $display("Q=%0d S=%0d DIV=%0d HDR=%h",
             Q, SEVEN, bw / Q, hdr);
  endtask
  initial begin run(); $finish; end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "Q=24 S=7 DIV=50 HDR=31000000"),
        "task-local localparam width wrong; got {:?}",
        out
    );
}

#[test]
fn block_local_localparam_is_full_width() {
    let src = r#"
module t;
  initial begin
    begin
      localparam K = 100;
      int r;
      r = 500 / K;              // 5, not x (K must be 100 not 0)
      $display("K=%0d R=%0d", K, r);
    end
    $finish;
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "K=100 R=5"),
        "block-local localparam width wrong; got {:?}",
        out
    );
}
