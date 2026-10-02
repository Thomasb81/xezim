//! §8.25 / A.4.1.1: a class may forward its own type parameter to a base
//! class whose base is that parameter by NAME, with the same name
//! (`class fwd_c #(type BASE) extends wrap_c #(.BASE(BASE))`, the shape of
//! tue's `tue_sequencer_base`). The named list was substituted as text
//! before it was placed by position, so `.BASE(BASE)` became
//! `.base_c(base_c)`, named no parameter, and the base fell back to its
//! default: no base constructor, no inherited members, a failing upcast.

use xezim::simulate;

const SRC: &str = r#"
class port_c; endclass
class base_c;
  int x = 5;
  port_c port;
  function new(); port = new(); endfunction
endclass
class cfg_c; endclass
class wrap_c #(type BASE = int, type CFG = cfg_c, type PCFG = CFG) extends BASE;
  function new(); super.new(); endfunction
endclass
class fwd_named_c #(type BASE = int) extends wrap_c #(.BASE(BASE));
  function new(); super.new(); endfunction
endclass
class fwd_pos_c #(type BASE = int) extends wrap_c #(BASE);
endclass
class fwd_other_c #(type B = int) extends wrap_c #(.BASE(B));
endclass
// two named levels, other parameters named alongside, one left to its default
class fwd_mid_c #(type BASE = int, type CFG = cfg_c) extends wrap_c #(.CFG(CFG), .BASE(BASE));
  function new(); super.new(); endfunction
endclass
class fwd_top_c #(type BASE = int) extends fwd_mid_c #(.BASE(BASE));
  function new(); super.new(); endfunction
endclass
module top;
  initial begin
    base_c b;
    fwd_named_c #(base_c) a = new();
    fwd_pos_c   #(base_c) p = new();
    fwd_other_c #(base_c) o = new();
    fwd_top_c   #(base_c) t = new();
    $display("named=%0d,%0d pos=%0d other=%0d two=%0d,%0d cast=%0d,%0d",
             a.x, a.port != null, p.x, o.x, t.x, t.port != null,
             $cast(b, a), $cast(b, t));
  end
endmodule
"#;

#[test]
fn type_param_forwarded_by_name_resolves_base() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(out, ["named=5,1 pos=5 other=5 two=5,1 cast=1,1"], "{out:?}");
}
