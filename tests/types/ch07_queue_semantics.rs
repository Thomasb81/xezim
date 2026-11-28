//! IEEE 1800-2023 §7.10 queue semantics:
//! - Empty `pop_front`/`pop_back` element type defaults (`int`, `logic`, `string`, `real`)
//! - `q.delete(32'bx)` is a no-op (§7.10.2.3); `q_2s[4'bx]` yields `0`, `q_4s[4'bx]` yields `'x`
//! - Self-referential `push_front(q[1])` evaluates argument before shifting (integral & `string`)
//! - Unpacked-struct queue `==` and `!=` comparisons
//! - Nested bounded queue `int nq[string][$:2]` enforces max bound 3
//! - §7.10.3 `ref` to queue element tracks element across `push_front` and detaches after `pop`/`delete`

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulation should succeed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_has(out: &[String], expected: &str) {
    assert!(
        out.iter().any(|l| l == expected),
        "missing {expected:?} in {out:?}"
    );
}

#[test]
fn queue_defaults_xz_self_push_struct_eq_nested_bounds_and_ref_lifetime() {
    let out = sim_lines(
        r#"
module top;
  typedef struct { int a; string s; } st_t;

  int qi[$];
  logic [7:0] ql[$];
  string sq[$];
  real rq_empty[$];
  st_t q1[$], q2[$], q3[$];
  int nq[string][$:2];
  int mq[$];

  task automatic bump_mod_ref(ref int r);
    mq.push_front(5);
    r = 99;
    mq.pop_front(); // removes 5; r still refers to mq[1] == 99
    if (r !== 99) $display("FAIL_ALIVE r=%0d", r);
    mq.delete(1);   // removes 99; r detaches
    r = 777;
  endtask

  task automatic check_local_ref(ref int r, ref int lq[$]);
    lq.push_front(5);
    r = 88;
    lq.pop_back();  // r sits at index 2; still valid
    if (lq[2] !== 88) $display("FAIL_LOC_ALIVE lq2=%0d", lq[2]);
    lq.delete(2);
    r = 555;
  endtask

  initial begin
    int pi;
    logic [7:0] pl, xl;
    string ps;
    real pr;
    int iq[$];
    int lq[$];

    // Empty pop defaults
    pi = qi.pop_front();
    pl = ql.pop_back();
    ps = sq.pop_front();
    pr = rq_empty.pop_back();
    $display("EP %0d %02x [%s] %0.1f", pi, pl, ps, pr);

    // X/Z delete no-op & 2-state vs 4-state X read
    qi = '{10, 20};
    qi.delete(32'bx);
    ql = '{8'h11, 8'h22};
    xl = ql[4'bx];
    $display("DX %0d %0d %0d %02x", qi.size(), qi[0], qi[4'bx], xl);

    // Self-referential push_front
    sq = '{"aa", "bb"};
    sq.push_front(sq[1]);
    iq = '{10, 20};
    iq.push_front(iq[1]);
    $display("PF %0d %s %s %s %0d %0d %0d %0d",
      sq.size(), sq[0], sq[1], sq[2], iq.size(), iq[0], iq[1], iq[2]);

    // Unpacked struct queue equality
    q1 = '{'{1, "x"}, '{2, "y"}};
    q2 = '{'{1, "x"}, '{2, "y"}};
    q3 = '{'{1, "x"}, '{2, "z"}};
    $display("EQ %0d %0d %0d %0d", q1 == q2, q1 != q3, q1 == q3, q1 != q2);

    // Nested bounded queue max bound 3 across push_back / push_front / insert
    nq["k"].push_back(1);
    nq["k"].push_back(2);
    nq["k"].push_back(3);
    nq["k"].push_back(4);
    $display("NB1 %0d %0d %0d %0d", nq["k"].size(), nq["k"][0], nq["k"][1], nq["k"][2]);
    nq["k"].push_front(9);
    $display("NB2 %0d %0d %0d %0d", nq["k"].size(), nq["k"][0], nq["k"][1], nq["k"][2]);
    nq["k"].insert(1, 8);
    $display("NB3 %0d %0d %0d %0d", nq["k"].size(), nq["k"][0], nq["k"][1], nq["k"][2]);

    // §7.10.3 ref lifetime (module-level and task-local)
    mq = '{10, 20, 30};
    bump_mod_ref(mq[1]);
    $display("MREF %0d %0d %0d", mq.size(), mq[0], mq[1]);

    lq = '{10, 20, 30};
    check_local_ref(lq[1], lq);
    $display("LREF %0d %0d %0d", lq.size(), lq[0], lq[1]);
  end
endmodule
"#,
    );
    assert_has(&out, "EP 0 xx [] 0.0");
    assert_has(&out, "DX 2 10 0 xx");
    assert_has(&out, "PF 3 bb aa bb 3 20 10 20");
    assert_has(&out, "EQ 1 1 0 0");
    assert_has(&out, "NB1 3 1 2 3");
    assert_has(&out, "NB2 3 9 1 2");
    assert_has(&out, "NB3 3 9 8 1");
    assert_has(&out, "MREF 2 10 30");
    assert_has(&out, "LREF 2 5 10");
}
