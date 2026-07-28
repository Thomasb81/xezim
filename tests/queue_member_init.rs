//! Queue member handling in classes: default initializers and queue returns.
//!
//! Two bugs fixed:
//! 1. Class member queue default initializers (`int q[$] = '{1,2,3}`) were
//!    silently ignored — the queue always started empty regardless of the
//!    inline initializer. (elaborate.rs now records the initializer for
//!    dimensioned members too; instantiation populates `<handle>#member`.)
//! 2. `return <queue_member>` from a class method resolved the bare member
//!    name through `resolve_hier_name` (which doesn't consult
//!    `instance_assoc_member`), so `pending_ret_collection` was never set
//!    and the caller received an empty queue. This broke UVM's
//!    `uvm_report_message_element_container::get_elements()`.

use xezim::simulate;

fn m(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

/// A queue member with an inline default initializer (`int q[$] = '{1,2,3}`)
/// must be populated at construction time, not left empty.
#[test]
fn queue_member_default_initializer() {
    let src = r#"
module top;
  class container;
    int elements[$] = '{10, 20, 30};
  endclass
  initial begin
    container c = new();
    $display("size %0d", c.elements.size());
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.iter().any(|s| s.contains("size 3")), "outs: {:?}", outs);
}

/// Assigning FROM a class member queue to a local must copy all elements.
#[test]
fn assign_from_queue_member() {
    let src = r#"
module top;
  class container;
    int elements[$] = '{10, 20, 30};
  endclass
  initial begin
    container c = new();
    int b[$];
    b = c.elements;
    $display("b.size %0d", b.size());
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.iter().any(|s| s.contains("b.size 3")), "outs: {:?}", outs);
}

/// Returning a queue member from a class method must propagate the elements.
/// This mirrors UVM's `get_elements()` pattern.
#[test]
fn return_queue_from_method() {
    let src = r#"
module top;
  class inner;
    int data[$];
    function void add(int v); data.push_back(v); endfunction
    function int get_size(); return data.size(); endfunction
  endclass
  initial begin
    inner obj = new();
    obj.add(1);
    obj.add(2);
    obj.add(3);
    $display("size %0d", obj.get_size());
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.iter().any(|s| s.contains("size 3")), "outs: {:?}", outs);
}

/// A typedef'd queue return type (UVM `queue_of_element` pattern) must work
/// when the method is called on a member object of another class.
#[test]
fn typedef_queue_return_from_member_method() {
    let src = r#"
module top;
  class base_elem;
    string name;
    function new(string n = ""); name = n; endfunction
  endclass
  typedef base_elem elem_q[$];
  class inner;
    base_elem data[$];
    function void add(base_elem e); data.push_back(e); endfunction
    function int get_size(); return data.size(); endfunction
    function elem_q get_data(); return data; endfunction
  endclass
  class outer;
    inner _inner;
    function new(); _inner = new(); endfunction
    function void do_print();
      base_elem rq[$];
      rq = _inner.get_data();
      $display("rq.size %0d", rq.size());
    endfunction
  endclass
  initial begin
    outer o = new();
    base_elem e1 = new("e1");
    base_elem e2 = new("e2");
    o._inner.add(e1);
    o._inner.add(e2);
    o.do_print();
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.iter().any(|s| s.contains("rq.size 2")), "outs: {:?}", outs);
}

/// Queue initialized via push_back in the constructor must also be readable
/// through a method return.
#[test]
fn queue_pushback_then_return() {
    let src = r#"
module top;
  class container;
    int elements[$];
    function new();
      elements.push_back(10);
      elements.push_back(20);
    endfunction
    function int get_first();
      return elements[0];
    endfunction
    function int get_second();
      return elements[1];
    endfunction
  endclass
  initial begin
    container c = new();
    $display("first %0d second %0d", c.get_first(), c.get_second());
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.iter().any(|s| s.contains("first 10 second 20")), "outs: {:?}", outs);
}
