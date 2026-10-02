//! §18.4 / §18.5.9: the objects held in a `rand` array of class handles are
//! randomized like a single rand handle, and the enclosing class may
//! constrain their members through an element select (`c[0].x == 6`,
//! `foreach (c[i]) c[i].x == …`). Every such randomize() used to fail:
//!   * the handles of a fixed `rand obj c[N]` were drawn as integers, so the
//!     elements pointed at no object after the first trial, and
//!   * an item on `c[k].fld` was neither pushed into element k's own solve
//!     nor repaired by the enclosing one, so it failed every trial.
//! The element handles themselves are never changed by randomize() (§18.4).
//! Non-rand members of the enclosing object are state variables of the
//! solve (§18.3), so `c[1].x == n` and `if (en) …` are solved as constants.

use xezim::simulate;

const SRC: &str = r#"
class leaf_c;
  rand int z;
endclass
class child_c;
  rand int x;
  rand int y;
  rand leaf_c sub;
  constraint c_y { y inside {[0:100]}; }
  function new(); sub = new(); endfunction
endclass
class fixed_c;
  rand child_c c[2];
  constraint k { c[0].x == 6; c[1].x inside {[3:5]}; c[1].y > 90; }
  function new(); c[0] = new(); c[1] = new(); endfunction
endclass
class one_c;
  rand child_c c[1];
  constraint k { c[0].x == 7; }
  function new(); c[0] = new(); endfunction
endclass
class foreach_c;
  rand child_c c[3];
  constraint k { foreach (c[i]) { c[i].x == i + 10; c[i].sub.z == 2 * i; } }
  function new(); foreach (c[i]) c[i] = new(); endfunction
endclass
class dyn_c;
  rand child_c d[];
  constraint k { d[1].x == 8; foreach (d[i]) d[i].y < 5; }
  function new(); d = new[2]; d[0] = new(); d[1] = new(); endfunction
endclass
class state_c;
  rand child_c c[2];
  int n;
  bit en;
  constraint k { c[1].x == n; if (en) c[0].y == 1; }
  function new(); c[0] = new(); c[1] = new(); endfunction
endclass
class unsat_c;
  rand child_c c[2];
  constraint k { c[0].x == 1; c[0].x == 2; }
  function new(); c[0] = new(); c[1] = new(); endfunction
endclass
module top;
  int fails, moved;
  initial begin
    fixed_c f = new();
    one_c o = new();
    foreach_c fe = new();
    dyn_c d = new();
    state_c t = new();
    unsat_c u = new();
    child_c h0, h1;
    for (int i = 0; i < 20; i++) begin
      h0 = f.c[0]; h1 = f.c[1];
      if (!f.randomize() || f.c[0].x != 6 || !(f.c[1].x inside {[3:5]}) || f.c[1].y <= 90
          || !(f.c[0].y inside {[0:100]})) fails++;
      if (f.c[0] != h0 || f.c[1] != h1) moved++;
      if (!o.randomize() || o.c[0].x != 7) fails++;
      if (!fe.randomize()) fails++;
      foreach (fe.c[j]) if (fe.c[j].x != j + 10 || fe.c[j].sub.z != 2 * j) fails++;
      if (!d.randomize() || d.d[1].x != 8 || d.d[0].y >= 5 || d.d[1].y >= 5) fails++;
      t.n = 20 + i; t.en = i[0];
      if (!t.randomize() || t.c[1].x != 20 + i || (t.en && t.c[0].y != 1)) fails++;
    end
    h0 = u.c[0]; h1 = u.c[1];
    $display("fails=%0d moved=%0d unsat=%0d unsat_moved=%0d", fails, moved, u.randomize(),
             u.c[0] != h0 || u.c[1] != h1);
  end
endmodule
"#;

#[test]
fn rand_obj_array_element_member_constraints() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(out, ["fails=0 moved=0 unsat=0 unsat_moved=0"], "{out:?}");
}
