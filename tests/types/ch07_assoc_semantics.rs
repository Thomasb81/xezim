//! IEEE 1800-2023 §7.8–§7.9 associative-array semantics:
//! - Wildcard `[*]` unsigned integer key normalization & wide (>64-bit) equivalence (#15)
//! - Full-range 64-bit unsigned (`>= 2^63`) `foreach` / `first` / `next` keys (#15)
//! - Wide (>64-bit) signed key ordering (`first` / `last` / `next` / `prev`) (#15)
//! - Unpacked struct AA keys (`exists`, `delete`, `first`, `next`, patterns, NBA) (#15)
//! - X/Z key handling (`exists`/`delete`/read) & 2-state X/Z->0 element coercion (#16)
//! - Member defaults on first struct-AA element write & `size() == num()` (#16, #17)
//! - `delete(k)` on AA of queues cleans nested queue storage (#17)
//! - §7.9.8 narrower integral locator arg & §7.9.11 `default:` literal semantics (#17)

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
fn assoc_keys_defaults_methods_and_nested_delete() {
    let out = sim_lines(
        r#"
module top;
  typedef bit signed [79:0] ws_t;
  typedef struct { int a; byte b; } ks_t;
  typedef struct { int x = 1; int y = 2; } xy_t;

  int aw[*];
  logic signed [3:0] k1 = -1;
  byte k2 = -1;

  bit [63:0] m64[bit [63:0]];
  int m80[ws_t];
  int mst[ks_t];
  logic [7:0] la[int];
  int a2[int];
  xy_t b[int];
  int aq[int][$];
  int m32[int];

  int cnt = 0;
  function automatic int inc_def();
    cnt++;
    return 50 + cnt;
  endfunction

  function automatic int read_def(int f[int]);
    return f[999];
  endfunction

  initial begin
    ws_t k80;
    ks_t ks1, ks2, ks3;
    bit [15:0] nk;
    int ad[int];
    int ok;

    // #15 wildcard unsigned & >64b equivalence & string literal
    aw[k1] = 10;
    aw[k2] = 20;
    aw[128'h41] = 30;
    $display("W %0d %0d %0d %0d", aw[15], aw[255], aw[32'h41], aw["A"]);

    // #15 bit [63:0] >= 2^63 in foreach
    m64[64'd5] = 22;
    m64[64'h8000_0000_0000_0001] = 11;
    foreach (m64[k]) $display("U64 %0h=%0d", k, m64[k]);

    // #15 signed >64-bit key order
    m80[-80'sd1] = 100;
    m80[80'sd5] = 200;
    void'(m80.first(k80));
    $display("S80 first=%0d %0d", k80, m80[k80]);
    void'(m80.last(k80));
    $display("S80 last=%0d %0d", k80, m80[k80]);

    // #15 unpacked struct key (variables, patterns, first/next, delete, NBA)
    ks1 = '{1, 2};
    ks2 = '{1, 3};
    ks3 = '{1, 2};
    mst[ks1] = 77;
    mst['{0, 9}] = 11;
    ok = mst.first(ks3);
    $display("STF %0d a=%0d b=%0d v=%0d", ok, ks3.a, ks3.b, mst[ks3]);
    ok = mst.next(ks3);
    $display("STN %0d a=%0d b=%0d v=%0d", ok, ks3.a, ks3.b, mst[ks3]);
    $display("ST ex1=%0d ex2=%0d exp=%0d", mst.exists('{1, 2}), mst.exists(ks2), mst.exists('{0, 9}));
    mst.delete('{0, 9});
    $display("STD num=%0d exp=%0d", mst.num(), mst.exists('{0, 9}));
    mst['{2, 4}] <= 88;
    mst[ks2] <= 99;
    ks2.a = 999;
    #1;
    $display("STNBA v24=%0d v13=%0d ex999=%0d", mst['{2, 4}], mst['{1, 3}], mst.exists('{999, 3}));

    // #16 X/Z exists/delete/read & 2-state coercion
    la[1] = 8'h5a;
    la.delete(4'bx);
    $display("XZ ex=%0d v=%02x num=%0d", la.exists(4'bx), la[4'bx], la.num());
    a2[3] = 32'hx5z3;
    $display("2S %08x", a2[3]);

    // #16/#17 struct member default + size()==num()
    b[2].x = 5;
    $display("SDEF %0d %0d %0d %0d", b.size(), b.num(), b[2].x, b[2].y);

    // #17 delete(k) on AA of queues
    aq[1].push_back(9);
    aq.delete(1);
    $display("DELQ %0d %0d", aq.num(), aq.exists(1));

    // #17 narrower integral arg to first/last/next/prev
    m32[32'h1_0002] = 9;
    $display("NAR %0d %0d", m32.first(nk), nk);

    // #17 literal overwrite, single-eval default, delete() preserves default, by-value formal
    ad = '{1: 10, default: inc_def()};
    $display("L1 num=%0d d=%0d cnt=%0d f=%0d", ad.num(), ad[777], cnt, read_def(ad));
    ad.delete();
    $display("L2 num=%0d d=%0d", ad.num(), ad[888]);
    ad = '{2: 20};
    $display("L3 num=%0d ex1=%0d v2=%0d d=%0d", ad.num(), ad.exists(1), ad[2], ad[777]);
  end
endmodule
"#,
    );
    assert_has(&out, "W 10 20 30 30");
    assert_has(&out, "U64 5=22");
    assert_has(&out, "U64 8000000000000001=11");
    assert_has(&out, "S80 first=-1 100");
    assert_has(&out, "S80 last=5 200");
    assert_has(&out, "STF 1 a=0 b=9 v=11");
    assert_has(&out, "STN 1 a=1 b=2 v=77");
    assert_has(&out, "ST ex1=1 ex2=0 exp=1");
    assert_has(&out, "STD num=1 exp=0");
    assert_has(&out, "STNBA v24=88 v13=99 ex999=0");
    assert_has(&out, "XZ ex=0 v=xx num=1");
    assert_has(&out, "2S 00000503");
    assert_has(&out, "SDEF 1 1 5 2");
    assert_has(&out, "DELQ 0 0");
    assert_has(&out, "NAR -1 2");
    assert_has(&out, "L1 num=1 d=51 cnt=1 f=51");
    assert_has(&out, "L2 num=0 d=51");
    assert_has(&out, "L3 num=1 ex1=0 v2=20 d=0");
}
