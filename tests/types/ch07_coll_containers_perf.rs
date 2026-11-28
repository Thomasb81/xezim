//! Native container & zero-preallocation regression tests (§7.5–§7.10):
//! - Empty multi-dimensional dynamic arrays (`int d[][][][]`) and wide fixed-outer
//!   dynamic arrays (`int arr[1024][]`) elaborate and simulate with zero pre-allocation
//! - Native `VecDeque` queues handle 10,000+ `push_front`/`pop_front` operations quickly
//! - Native `BTreeMap` associative arrays handle 10,000+ insert/lookup/`first`/`next`/`num`/`delete`

use xezim::simulate;

#[test]
fn zero_prealloc_multidim_dynamic_and_large_queue_assoc_ops() {
    let src = r#"
module top;
  int d4[][][][];
  int arr[1024][];
  int q[$];
  int aa[int];

  initial begin
    int i, k, cnt;
    longint sum_q, sum_aa;
    int first_val, last_val;

    // 1. Multi-dim & large fixed-outer empty dynamic arrays do not pre-allocate 64^k slots
    if (d4.size() != 0 || arr[0].size() != 0 || arr[1023].size() != 0)
      $display("TAG_FAIL_SIZE d4=%0d a0=%0d a1023=%0d", d4.size(), arr[0].size(), arr[1023].size());
    arr[500] = new[2];
    arr[500][0] = 17;
    arr[500][1] = 29;
    $display("PREALLOC %0d %0d %0d %0d", d4.size(), arr[0].size(), arr[500][0], arr[500][1]);

    // 2. 10,000+ push_front and partial pop_front on native VecDeque queue
    for (i = 0; i < 10000; i++) q.push_front(i);
    if (q.size() != 10000 || q[0] != 9999 || q[9999] != 0)
      $display("TAG_FAIL_QPUSH sz=%0d h=%0d t=%0d", q.size(), q[0], q[9999]);
    sum_q = 0;
    for (i = 0; i < 5000; i++) sum_q += q.pop_front();
    first_val = q[0];
    last_val = q[$];
    $display("QPERF sz=%0d h=%0d t=%0d sum=%0d", q.size(), first_val, last_val, sum_q);

    // 3. 10,000+ insert, lookup, ordered first/next, num, delete on native BTreeMap AA
    for (i = 0; i < 10000; i++) aa[i * 3 - 5000] = i + 1;
    cnt = 0;
    sum_aa = 0;
    if (aa.first(k)) begin
      do begin
        cnt++;
        sum_aa += aa[k];
      end while (aa.next(k));
    end
    for (i = 0; i < 1000; i++) aa.delete(i * 3 - 5000);
    $display("AAPERF num=%0d cnt=%0d sum=%0d ex0=%0d ex1000=%0d v=%0d",
      aa.num(), cnt, sum_aa, aa.exists(-5000), aa.exists(1000 * 3 - 5000), aa[1000 * 3 - 5000]);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should succeed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    // sum_{i=5000..9999} i = 5000 * (5000 + 9999) / 2 = 37497500
    assert!(
        out.iter().any(|l| l == "PREALLOC 0 0 17 29"),
        "missing PREALLOC in {out:?}"
    );
    assert!(
        out.iter().any(|l| l == "QPERF sz=5000 h=4999 t=0 sum=37497500"),
        "missing QPERF in {out:?}"
    );
    // sum_{i=0..9999} (i + 1) = 10000 * 10001 / 2 = 50005000
    assert!(
        out.iter().any(|l| l == "AAPERF num=9000 cnt=10000 sum=50005000 ex0=0 ex1000=1 v=1001"),
        "missing AAPERF in {out:?}"
    );
}
