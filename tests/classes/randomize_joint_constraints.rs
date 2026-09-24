//! §18 — constraint mechanisms cross-checked against the reference
//! simulator. Assertions are seed-independent (counts and invariants, not
//! values), so they hold for any conforming solver.
//!
//! The `#[ignore]`d test is a feasible problem the reference solves on every
//! call and xezim's solver cannot: arithmetic (`sum()`), `unique` and an
//! ordering chain over one array must be solved jointly. Propagation plus
//! rejection sampling returns 0 from every `randomize()` today.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn field(lines: &[String], key: &str) -> i64 {
    let line = lines
        .iter()
        .find(|l| l.contains(key))
        .unwrap_or_else(|| panic!("missing {key}: {lines:?}"));
    let rest = &line[line.find(key).unwrap() + key.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn supported_constraint_mechanisms_hold() {
    let o = out(r#"
class mix;
  rand bit [3:0] x; rand bit [3:0] y; randc bit [2:0] rc; rand bit en;
  constraint c_sb { solve en before x; }
  constraint c_imp { en -> x == 0; }
  constraint c_soft { soft y == 7; }
  constraint c_dist { en dist {1 := 1, 0 := 9}; }
endclass
module tb;
  initial begin
    automatic mix m = new;
    automatic int en1 = 0, soft7 = 0, bad = 0, r;
    automatic int rc_hist[8];
    for (int i = 0; i < 800; i++) begin
      void'(m.randomize());
      if (m.en) begin en1++; if (m.x != 0) bad++; end
      if (m.y == 7) soft7++;
      rc_hist[m.rc]++;
    end
    $display("EN1=%0d SOFT7=%0d BAD=%0d", en1, soft7, bad);
    foreach (rc_hist[i]) if (rc_hist[i] != 100) $display("RANDC_UNEVEN %0d=%0d", i, rc_hist[i]);
    r = m.randomize() with { y == 3; x > 5; };
    $display("INLINE R=%0d Y=%0d XGT5=%0d", r, m.y, m.x > 5);
    r = m.randomize() with { x > 20; };
    $display("INFEASIBLE R=%0d", r);
  end
endmodule
"#);
    // `dist` 1:9 over 800 draws: mean 80, sd ~8.5.
    let en1 = field(&o, "EN1=");
    assert!((40..=130).contains(&en1), "dist weighting off: {o:?}");
    assert_eq!(
        field(&o, "SOFT7="),
        800,
        "an unopposed soft constraint must hold: {o:?}"
    );
    assert_eq!(field(&o, "BAD="), 0, "implication violated: {o:?}");
    assert!(
        !o.iter().any(|l| l.contains("RANDC_UNEVEN")),
        "randc must cycle through all 8 values every 8 draws: {o:?}"
    );
    assert!(
        o.iter().any(|l| l.contains("INLINE R=1 Y=3 XGT5=1")),
        "{o:?}"
    );
    assert!(o.iter().any(|l| l.contains("INFEASIBLE R=0")), "{o:?}");
}

#[test]
#[ignore = "solver cannot satisfy sum() + unique + ordering jointly (fix pending: needs a real solver)"]
fn arithmetic_unique_and_ordering_solve_jointly() {
    let o = out(r#"
class hard;
  rand bit [7:0] a[12];
  constraint c1 { foreach (a[i]) a[i] inside {[1:30]}; }
  constraint c2 { a.sum() with (int'(item)) == 200; }
  constraint c3 { unique {a}; }
  constraint c4 { foreach (a[i]) if (i > 0) a[i] > a[i-1]; }
endclass
module tb;
  initial begin
    automatic hard h = new;
    automatic int ok = 0, valid = 0;
    for (int i = 0; i < 50; i++) begin
      if (h.randomize()) begin
        automatic int s = 0; automatic bit good = 1;
        ok++;
        foreach (h.a[j]) s += h.a[j];
        for (int j = 1; j < 12; j++) if (h.a[j] <= h.a[j-1]) good = 0;
        if (s == 200 && good) valid++;
      end
    end
    $display("OK=%0d VALID=%0d", ok, valid);
  end
endmodule
"#);
    assert!(o.iter().any(|l| l.contains("OK=50 VALID=50")), "{o:?}");
}
