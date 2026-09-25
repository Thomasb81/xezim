//! §18.5.4 — `dist` weights. Each test draws a few thousand solutions and
//! checks the weight ratios with bounds of about four standard deviations;
//! every expectation was cross-checked against the reference simulator.

use xezim::simulate;

fn counts(src: &str) -> Vec<(String, i64)> {
    let sim = simulate(src, 100).expect("simulate failed");
    let mut out = Vec::new();
    for o in &sim.output {
        for tok in o.message.split_whitespace() {
            if let Some((k, v)) = tok.split_once('=') {
                if let Ok(n) = v.parse() {
                    out.push((k.to_string(), n));
                }
            }
        }
    }
    out
}

fn get(c: &[(String, i64)], k: &str) -> i64 {
    c.iter()
        .find(|(n, _)| n == k)
        .unwrap_or_else(|| panic!("missing {k}: {c:?}"))
        .1
}

fn within(c: &[(String, i64)], k: &str, lo: i64, hi: i64) {
    let v = get(c, k);
    assert!(
        (lo..=hi).contains(&v),
        "{k}={v} outside [{lo}, {hi}]: {c:?}"
    );
}

/// A dist per `if` branch on the same variable: each branch keeps its own
/// weights. The per-variable trials dealt one schedule per variable and
/// restarted it whenever the other branch's weights came up, and a fresh
/// schedule opens on its heaviest item, so both branches over-drew their
/// heavy value (0.82 instead of 0.75, 0.84 instead of 0.80). The `randc`
/// member keeps this class on the per-variable trials.
#[test]
fn dist_per_branch_keeps_its_weights() {
    let c = counts(
        r#"
class F4;
  randc bit [1:0] rc;
  rand bit m;
  rand bit [3:0] v;
  constraint c { if (m) v dist {0 := 3, 1 := 1}; else v dist {[8:11] :/ 1, 15 := 4}; }
endclass
module top;
  initial begin
    automatic F4 o = new;
    automatic int m1, v0, v1, v15, v8, other, f;
    for (int i = 0; i < 4000; i++) begin
      if (!o.randomize()) f++;
      if (o.m) begin m1++; if (o.v == 0) v0++; else if (o.v == 1) v1++; else other++; end
      else begin if (o.v == 15) v15++; else if (o.v == 8) v8++; else if (o.v < 8 || o.v > 11) other++; end
    end
    $display("fails=%0d other=%0d m1=%0d v0p=%0d v15p=%0d v8p=%0d", f, other, m1,
             v0 * 1000 / m1, v15 * 1000 / (4000 - m1), v8 * 1000 / (4000 - m1));
  end
endmodule
"#,
    );
    assert_eq!(get(&c, "fails"), 0, "{c:?}");
    assert_eq!(get(&c, "other"), 0, "{c:?}");
    // per mille: 750 (sd ~10), 800 (sd ~9), 50 (sd ~5)
    within(&c, "v0p", 710, 790);
    within(&c, "v15p", 765, 835);
    within(&c, "v8p", 30, 70);
}
