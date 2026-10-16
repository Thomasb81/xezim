//! Issue #327: an `output` area array formal whose actual argument comes from
//! a caller formal filled from its DEFAULT value was truncated to 1 bit per
//! element (each element came back holding only its LSB, e.g. `8'hA5` read as
//! `1`), and a prior bound-actual array call to any callee "warmed" the path so
//! the same code then worked. Root cause: when an unpacked-array formal is
//! initialised from its default, it was never materialised as named element
//! signals nor registered in the array table, so a later output/ref call that
//! bound that formal as its caller actual dropped the write-back.
//!
//! The fix materialises the default-valued unpacked-array formal exactly like a
//! bound array actual (`bind_array_arg`): named `v[i]` element signals at the
//! element width/signedness + a `module.arrays` entry. Expected lines come from
//! the reference simulator.

use xezim::simulate;

#[test]
fn output_array_formal_default_writeback_is_not_truncated() {
    let out: Vec<String> = simulate(
        r#"
module top;
  // Variant A: 2-state byte, the issue's reproducer shape.
  function automatic void ia(output byte v[2]); v[0] = 8'hA5; v[1] = 8'h5A; endfunction
  function automatic void ia_default(byte v[2] = '{8'h00, 8'h00});
    ia(v);
    $display("T|IA %h %h %0d %0d", v[0], v[1], v[0], v[1]);
  endfunction
  // Variant B: string elements (the issue mentions "AA" truncating to "A").
  function automatic void fs(output string v[2]); v[0] = "AA"; v[1] = "BB"; endfunction
  function automatic void fs_default(string v[2] = '{ "", "" });
    fs(v);
    $display("T|FS '%s' '%s'", v[0], v[1]);
  endfunction
  // Variant C: an UNPACKED-struct array (member leaves). The default leaves
  // must keep the caller's DECLARED member signedness so `%0d` on a `byte`
  // member prints -16, not 240, and the callee's output reaches the caller.
  typedef struct { byte a; byte b; } pair_t;
  function automatic void psd(output pair_t v[2]); v[0].a = 8'hF0; v[0].b = 8'h10; v[1].a = 8'h80; v[1].b = 8'h01; endfunction
  function automatic void ps_default(pair_t v[2] = '{'{8'h00,8'h00},'{8'h00,8'h00}});
    psd(v);
    $display("T|PS %0d %0d %0d %0d", v[0].a, v[0].b, v[1].a, v[1].b);
  endfunction
  initial begin
    $display("T|COLD");
    ia_default();
    fs_default();
    ps_default();
    // A prior bound-actual call to a same-shaped callee used to "warm" the
    // path; it must make no difference to the (now correct) write-back.
    $display("T|AFTER-BOUND");
    begin byte q[2]; ia(q); end
    ia_default();
    fs_default();
    ps_default();
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .filter(|m| m.starts_with("T|"))
    .collect();
    let want = [
        "T|COLD",
        "T|IA a5 5a -91 90",
        "T|FS 'AA' 'BB'",
        "T|PS -16 16 -128 1",
        "T|AFTER-BOUND",
        "T|IA a5 5a -91 90",
        "T|FS 'AA' 'BB'",
        "T|PS -16 16 -128 1",
    ];
    assert_eq!(out, want, "{out:?}");
}