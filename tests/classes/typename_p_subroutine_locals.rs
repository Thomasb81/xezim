//! `%p` must render a SUBROUTINE-local variable's real value, and a string
//! flowing through a TYPE PARAMETER (generic class method) must not be
//! truncated. UVM's resource-class overrides (`uvm_resource#(T)`'s
//! `do_read`/`do_write`) use `T _t` locals in a `%p`; pre-fix `%p` printed `x`/empty for any function-local (its
//! value is not a module signal), and a `T`-local/return was truncated to
//! `_t`'s wrong declared width, so `T=string` read back 4 chars ("rld!")
//! instead of the full text and the test's `m.read()` VALUE assertion failed
//! (`val != VAL`).
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/virtual_rw_p.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/virtual_rw_p.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"class res #(parameter type T = int);
  protected T val;
  virtual function T do_read();
    return val;
  endfunction
  virtual function void do_write(T t);
    val = t;
  endfunction
  function T read();
    return do_read();
  endfunction
  function void write(T t);
    do_write(t);
  endfunction
endclass

class my_res #(parameter type T = string) extends res#(T);
  virtual function void do_write(T t);
    super.do_write(t);
  endfunction
  virtual function T do_read();
    T _t;
    _t = super.do_read();
    $display("READ-P=%p", _t);
    return _t;
  endfunction
endclass

module top;
  int a;
  string s;
  my_res#(string) r;
  initial begin
    // %p on ordinary subroutine-locals (pre-fix: x / empty)
    a = 42;
    s = "hello";
    $display("A=%p S=%p", a, s);
    // string through a type-parameterized virtual method (pre-fix: "rld!")
    r = new;
    r.write("Hello, world!");
    $display("RES=%s", r.read());
    $finish;
  end
endmodule
"#;

#[test]
fn typename_p_formats_subroutine_locals() {
    let out = run(SRC);
    assert!(
        out.contains("A=42") && out.contains("S=\"hello\""),
        "`%p` must render subroutine-locals with their real value; expected \
         `A=42 S=\"hello\"`, got:\n{out}"
    );
    assert!(
        out.contains("READ-P=\"Hello, world!\"") && out.contains("RES=Hello, world!"),
        "a `string` through `T` (generic virtual method) must not be \
         truncated; expected the full `Hello, world!`, got:\n{out}"
    );
}