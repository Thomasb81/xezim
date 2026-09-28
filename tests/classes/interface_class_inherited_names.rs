//! §8.26.6.2 / §8.26.6.3: an interface class that inherits one parameter
//! name from two different base classes, or from two specializations of one
//! class, must declare that name itself. The reference simulator rejects both
//! and accepts the diamond through one specialization.

use xezim::simulate;

#[test]
fn conflicting_inherited_type_parameter() {
    let src = r#"
module class_tb ();
  interface class ic1#(type T = logic);
    pure virtual function void fn1(T a);
  endclass
  interface class ic2#(type T = logic);
    pure virtual function void fn2(T a);
  endclass
  interface class ic3#(type TYPE = logic) extends ic1#(TYPE), ic2#(TYPE);
  endclass
endmodule
"#;
    assert!(simulate(src, 10).is_err());
    let src = r#"
module class_tb ();
  interface class ibase#(type T = logic);
    pure virtual function void fn(T val);
  endclass
  interface class ic1 extends ibase#(bit);
    pure virtual function void fn1();
  endclass
  interface class ic2 extends ibase#(string);
    pure virtual function void fn2();
  endclass
  interface class ic3 extends ic1, ic2;
    pure virtual function void fn3();
  endclass
endmodule
"#;
    assert!(simulate(src, 10).is_err());
}

#[test]
fn resolved_inherited_type_parameter() {
    let src = r#"
module class_tb ();
  interface class ibase#(type T = logic);
    pure virtual function void fn(T val);
  endclass
  interface class ic1 extends ibase#(bit);
    pure virtual function void fn1();
  endclass
  interface class ic2 extends ibase#(bit);
    pure virtual function void fn2();
  endclass
  interface class ic3 extends ic1, ic2;
    pure virtual function void fn3();
  endclass
  interface class ic4#(type T = int) extends ic1, ibase#(string);
  endclass
  class impl implements ic3;
    virtual function void fn(bit val); $display("I|%b", val); endfunction
    virtual function void fn1(); endfunction
    virtual function void fn2(); endfunction
    virtual function void fn3(); endfunction
  endclass
  initial begin
    impl o = new;
    o.fn(1'b1);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("one specialization is legal");
    assert!(
        sim.output.iter().any(|o| o.message == "I|1"),
        "{:?}",
        sim.output
    );
}
