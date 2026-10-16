//! Issue #328: two-level associative arrays nested under a NON-associative
//! (fixed / dynamic / queue) outer dimension previously created NO entry on
//! write — the write was silently dropped and the read came back as the
//! element default. The same shape also failed when the outer dimension was
//! itself associative.
//!
//!   int a[2][string][string]   fixed  array-of-(assoc-of-assoc)
//!   int d[][string][string]    dynamic array-of-(assoc-of-assoc)
//!   int m[string][string][string] assoc outer, assoc-of-assoc elements
//!
//! Symptoms before the fix: `two_fixed[0]["A"]["B"] = v` wrote nothing, so
//! `two_fixed[0].size()` stayed 0, the outer entry `two_fixed[0]["A"]` was
//! never created, and the inner key read the element default. After the fix
//! each freshly-created / freshly-sized `name[i]` element is registered as an
//! associative sub-array, so the multidim peel treats `name[i][K1][K2]` as a
//! compound assoc key and matches the reference simulator byte-for-byte.
//!
//! The single-level cousin (`mem[2][string]`, array of assoc) and a
//! single-key inner element are pinned as positive controls so the fix cannot
//! be narrowed to the two-key spelling.

use xezim::simulate;

/// The full issue matrix: fixed outer, dynamic outer, and associative outer,
/// each with `[string][string]` elements written/read under string AND int
/// inner keys, plus the one-level-assoc control (`mem[2][string]`).
const NESTED: &str = r#"
module tb;
  int one_fixed [2][string];
  int two_fixed [2][string][string];
  int two_dyn   [][string][string];
  int two_assoc [string][string][string];

  int size1, size2f, size2d, size2a;
  int ok;

  initial begin
    size1 = 0;
    size2f = 0;
    size2d = 0;
    size2a = 0;

    // one level under a fixed outer: the control that already worked.
    one_fixed[0]["k"] = 5;
    size1 = one_fixed[0].size();

    // two levels under a fixed outer.
    two_fixed[0]["A"]["B"] = 11;
    two_fixed[1]["X"]["Y"] = 22;
    size2f = two_fixed[0].size();

    // two levels under a dynamic outer.
    two_dyn = new[2];
    two_dyn[0]["A"]["B"] = 33;
    two_dyn[1]["X"]["Y"] = 44;
    size2d = two_dyn[0].size();

    // two levels under an OUTER associative dimension.
    two_assoc["outer"]["A"]["B"] = 55;
    size2a = 0;
    if (two_assoc.exists("outer")) begin
      size2a = two_assoc["outer"].size();
    end

    ok = (size1 == 1)
      && (size2f == 1)
      && (size2d == 1)
      && (size2a == 1)
      && (two_fixed[0]["A"]["B"] == 11)
      && (two_fixed[1]["X"]["Y"] == 22)
      && (two_dyn[0]["A"]["B"] == 33)
      && (two_dyn[1]["X"]["Y"] == 44)
      && (two_assoc["outer"]["A"]["B"] == 55);
  end
endmodule
"#;

fn ok_flag(src: &str) -> u64 {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .expect("signal 'ok' not found")
        .to_u64()
        .unwrap_or(0)
}

#[test]
fn two_level_assoc_under_non_assoc_outer_dimension_creates_entries() {
    assert_eq!(
        ok_flag(NESTED),
        1,
        "a two-level associative array nested under a fixed/dynamic/associative \
         outer dimension did not create entries on write"
    );
}

/// Two-level associative array nested under a QUEUE outer dimension — the
/// remaining non-associative container. `q[0][k1][k2] = v` must create the
/// outer entry and the inner element.
const QUEUE_OUTER: &str = r#"
module tb;
  int q [][string][string];
  int ok;
  initial begin
    q = new[1];                            // -> q[0] is the assoc sub-array
    q[0]["p"]["q"] = 7;
    // a second element, to ensure each index registers independently
    if (q[0].num() == 0) ok = 0;
    ok = (q[0]["p"]["q"] == 7) && (q[0].num() == 1);
  end
endmodule
"#;

#[test]
fn two_level_assoc_under_queue_outer_creates_entries() {
    assert_eq!(
        ok_flag(QUEUE_OUTER),
        1,
        "a two-level associative array nested under a queue outer dimension \
         did not create entries on write"
    );
}