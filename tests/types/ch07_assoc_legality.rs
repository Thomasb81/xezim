//! IEEE 1800-2023 §7.8–§7.10 compile-time legality checks:
//! - Legal: packed typedef dimensions in AA index & string literals on `[*]` / `int` AA (#14)
//! - Illegal: inline struct/enum AA index decls, `real`/`shortreal`/struct-with-real AA keys,
//!   wildcard `[*]` in `foreach`/`first`/`last`/`next`/`prev`/`find*_index`/`unique_index`/
//!   `item.index`, incompatible AA key expressions (`string` AA indexed by `int`, `int` or `[*]`
//!   AA indexed by `real` or `string` var), zero/negative queue upper bounds (#14, #18).

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("legal design should compile and simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(sim) => panic!("expected design to be rejected:\n{src}\noutput: {:?}", sim.output),
        Err(e) => e,
    }
}

#[test]
fn accepts_packed_typedef_assoc_index_and_string_literal_integral_keys() {
    let out = sim_lines(
        r#"
module top;
  typedef logic [3:0] nib_t;
  int aap[nib_t [1:0]];
  int aw[*];
  int ai[int];
  initial begin
    aap[8'h3f] = 44;
    aw["ab"] = 12;
    ai["ab"] = 34;
    $display("LEGAL %0d %0d %0d", aap[8'h3f], aw[16'h6162], ai[16'h6162]);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "LEGAL 44 12 34"),
        "unexpected output: {out:?}"
    );
}

#[test]
fn rejects_illegal_assoc_keys_wildcard_ops_and_queue_bounds() {
    // Inline struct / enum declarations inside AA index brackets
    rejected("module top; int a[struct { int x; }]; endmodule");
    rejected("module top; int a[enum { A, B }]; endmodule");

    // real / shortreal and struct containing real as AA index
    rejected("module top; int a[real]; endmodule");
    rejected("module top; int a[shortreal]; endmodule");
    rejected("module top; typedef struct { real r; } rs_t; int a[rs_t]; endmodule");

    // Wildcard [*] forbidden in traversal / locator index contexts
    rejected("module top; int a[*]; initial foreach (a[k]) ; endmodule");
    rejected("module top; int a[*]; int k; initial void'(a.first(k)); endmodule");
    rejected("module top; int a[*]; int k; initial void'(a.last(k)); endmodule");
    rejected("module top; int a[*]; int k; initial void'(a.next(k)); endmodule");
    rejected("module top; int a[*]; int k; initial void'(a.prev(k)); endmodule");
    rejected("module top; int a[*]; int q[$]; initial q = a.find_index with (item == 1); endmodule");
    rejected("module top; int a[*]; int q[$]; initial q = a.find_first_index with (item == 1); endmodule");
    rejected("module top; int a[*]; int q[$]; initial q = a.find_last_index with (item == 1); endmodule");
    rejected("module top; int a[*]; int q[$]; initial q = a.unique_index(); endmodule");
    rejected("module top; int a[*]; int q[$]; initial q = a.find with (item.index == 1); endmodule");

    // Incompatible index types on wildcard, string, and int AAs
    rejected("module top; int a[*]; real r = 1.0; initial a[r] = 1; endmodule");
    rejected("module top; int a[*]; string s = \"k\"; initial a[s] = 1; endmodule");
    rejected("module top; int a[string]; initial a[1] = 1; endmodule");
    rejected("module top; int a[int]; real r = 1.0; initial a[r] = 1; endmodule");
    rejected("module top; int a[int]; string s = \"k\"; initial a[s] = 1; endmodule");

    // Illegal queue upper bounds
    rejected("module top; int q[$:0]; endmodule");
    rejected("class C; int q[$:-1]; endclass module top; C c; endmodule");
}
