//! §15.4: `mb[i] = new()` on an ARRAY of mailboxes must actually allocate.
//!
//! `lvalue_container_kind` recognised only a bare `Ident` (or `this.member`)
//! as a mailbox/semaphore target, so an INDEXED lvalue fell through and the
//! `new()` was never treated as constructing a container. No mailbox was
//! registered for the handle it went on to store, so reads found a
//! live-looking (non-null) handle with nothing behind it: every `put`
//! silently vanished, `num()` stayed 0 no matter how many were queued, and
//! `try_get` always failed.
//!
//! The shape this came from is a BFM with `mailbox req_mb[N-1:0]`, one per
//! client, allocated in a loop. Requests were mailed and never arrived, so
//! the DUT looked like it had stopped responding rather than like a
//! collections bug — a scalar mailbox in the same scope worked fine.
//!
//! Neither the array nor its elements carry a `type_name` on their signals
//! (only scalars do), so the fix reads the ARRAY's declared type from the
//! declaration table.

use xezim::simulate;

fn run(src: &str) -> String {
    let sim = simulate(src, 20000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn new_into_a_mailbox_array_element_allocates_it() {
    // Reference-verified: 1 / 1 42 / 0.
    let out = run(r#"
module sub (input logic clk);
  mailbox #(int) mb [2];
  initial begin
    int got; int ok;
    mb[0] = new();
    @(posedge clk);
    mb[0].put(42);
    $display("PUT num=%0d", mb[0].num());
    ok = mb[0].try_get(got);
    $display("GET ok=%0d got=%0d", ok, got);
    $display("END num=%0d", mb[0].num());
  end
endmodule
module tb;
  logic clk = 0;
  always #500 clk = ~clk;
  sub u (.clk(clk));
  initial begin repeat (3) @(posedge clk); $finish; end
endmodule
"#);
    assert!(out.contains("PUT num=1"), "put did not land in the mailbox:\n{out}");
    assert!(out.contains("GET ok=1 got=42"), "try_get did not retrieve it:\n{out}");
    assert!(out.contains("END num=0"), "mailbox not drained:\n{out}");
}

#[test]
fn mailbox_array_elements_are_independent_and_accumulate() {
    // Each element must be its OWN mailbox: the original bug made every
    // element read as empty, so a test that only checked one could pass
    // against a single shared box.
    let out = run(r#"
module sub (input logic clk);
  mailbox #(int) mb [3];
  initial begin
    for (int i = 0; i < 3; i++) mb[i] = new();
    @(posedge clk);
    mb[0].put(1); mb[0].put(2); mb[0].put(3);
    mb[1].put(9);
    $display("N %0d %0d %0d", mb[0].num(), mb[1].num(), mb[2].num());
  end
endmodule
module tb;
  logic clk = 0;
  always #500 clk = ~clk;
  sub u (.clk(clk));
  initial begin repeat (3) @(posedge clk); $finish; end
endmodule
"#);
    assert!(out.contains("N 3 1 0"), "elements are not independent mailboxes:\n{out}");
}

#[test]
fn a_mailbox_array_inside_a_generate_block_works() {
    // The reported design allocated per-client mailboxes and used them from
    // a genvar-indexed generate block.
    let out = run(r#"
module sub #(parameter int N = 2) (input logic clk);
  mailbox #(int) mb [N-1:0];
  initial for (int i = 0; i < N; i++) mb[i] = new();
  genvar gi;
  generate for (gi = 0; gi < N; gi++) begin : client
    initial begin
      @(posedge clk); #1;
      mb[gi].put(100 + gi);
      $display("G%0d num=%0d", gi, mb[gi].num());
    end
  end endgenerate
endmodule
module tb;
  logic clk = 0;
  always #500 clk = ~clk;
  sub #(.N(2)) u (.clk(clk));
  initial begin repeat (3) @(posedge clk); $finish; end
endmodule
"#);
    assert!(out.contains("G0 num=1"), "genvar-indexed mailbox 0 empty:\n{out}");
    assert!(out.contains("G1 num=1"), "genvar-indexed mailbox 1 empty:\n{out}");
}

#[test]
fn a_semaphore_array_element_allocates_too() {
    // Same lvalue path serves semaphores; `new(n)` carries a key count.
    let out = run(r#"
module sub (input logic clk);
  semaphore sem [2];
  initial begin
    int ok;
    sem[0] = new(2);
    @(posedge clk);
    ok = sem[0].try_get(1);
    $display("S1 ok=%0d", ok);
    ok = sem[0].try_get(1);
    $display("S2 ok=%0d", ok);
    ok = sem[0].try_get(1);
    $display("S3 ok=%0d", ok);
  end
endmodule
module tb;
  logic clk = 0;
  always #500 clk = ~clk;
  sub u (.clk(clk));
  initial begin repeat (3) @(posedge clk); $finish; end
endmodule
"#);
    // Two keys available, so the third attempt must fail.
    assert!(out.contains("S1 ok=1"), "semaphore array element not allocated:\n{out}");
    assert!(out.contains("S2 ok=1"), "second key not available:\n{out}");
    assert!(out.contains("S3 ok=0"), "semaphore handed out more keys than it had:\n{out}");
}
