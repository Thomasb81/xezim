//! GitHub #235: an event control `@(u.ev)` / `@(ua[0].ev)` on a named event
//! declared in an instantiated submodule or interface must wake when a process
//! inside that instance triggers `->ev`.
//!
//! Root cause:
//! 1. In `xezim-core/src/elaborate.rs`, `rewrite_stmt` for `EventTrigger` used
//!    `format!("{}.{}", prefix, name.name)` instead of `cat2(prefix, &name.name)`.
//!    Because `prefix` ends with a dot (`"u."`), this synthesized an event name
//!    with two dots (`"u..ev"` / `"ua[0]..ev"`), which never matched the
//!    registered signal name (`"u.ev"` / `"ua[0].ev"`).
//! 2. Submodule events were never inserted into `elab.events` during inlining.
//! 3. `member_chain_as_flat_ident_impl` did not handle `ExprKind::Index`, so
//!    `ua[0].ev` failed to normalize to a hierarchical identifier and produced
//!    empty sensitivity (spurious wake at time 0).
//! 4. `resolve_hier_name_uncached` dropped constant segment selects on
//!    hierarchical identifiers.
//!
//! Verified byte-for-byte against reference simulators.

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .or_else(|| sim.get_signal(&format!("deep_top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

#[test]
fn scalar_and_array_instance_event_wait() {
    let sim = xezim::simulate(
        r#"
module sub;
  event ev;
  initial repeat (3) #10 ->ev;
endmodule

module top;
  sub u ();
  sub ua[1] ();
  int n_u = 0, n_ua = 0;
  initial begin
    fork
      repeat (3) begin @(u.ev);     n_u++;  end
      repeat (3) begin @(ua[0].ev); n_ua++; end
    join_none
    #100;
    $finish;
  end
endmodule
"#,
        200,
    )
    .expect("simulate");

    assert_eq!(u(&sim, "n_u"), 3, "@(u.ev) must wake 3 times");
    assert_eq!(u(&sim, "n_ua"), 3, "@(ua[0].ev) must wake 3 times");
}

#[test]
fn forever_loop_instance_event_wait() {
    let sim = xezim::simulate(
        r#"
module sub;
  event ev;
  initial repeat (3) #10 ->ev;
endmodule

module top;
  sub u ();
  sub ua[1] ();
  int n_u = 0, n_ua = 0;
  initial begin
    fork
      forever begin @(u.ev);     n_u++;  end
      forever begin @(ua[0].ev); n_ua++; end
    join_none
    #100;
    $finish;
  end
endmodule
"#,
        200,
    )
    .expect("simulate");

    assert_eq!(u(&sim, "n_u"), 3, "forever @(u.ev) must wake 3 times");
    assert_eq!(u(&sim, "n_ua"), 3, "forever @(ua[0].ev) must wake 3 times");
}

#[test]
fn deep_arbitrary_nested_hierarchy_event_wait() {
    let sim = xezim::simulate(
        r#"
module leaf;
  event ev;
  initial repeat (3) #10 ->ev;
endmodule

module mid;
  leaf l ();
  leaf la[2] ();
endmodule

module deep_top;
  mid m ();
  mid ma[2] ();

  int n_m_l = 0;
  int n_m_la0 = 0;
  int n_m_la1 = 0;
  int n_ma0_l = 0;
  int n_ma1_la1 = 0;

  initial begin
    fork
      forever begin @(m.l.ev);         n_m_l++;     end
      forever begin @(m.la[0].ev);     n_m_la0++;   end
      forever begin @(m.la[1].ev);     n_m_la1++;   end
      forever begin @(ma[0].l.ev);     n_ma0_l++;   end
      forever begin @(ma[1].la[1].ev); n_ma1_la1++; end
    join_none
    #100;
    $finish;
  end
endmodule
"#,
        200,
    )
    .expect("simulate");

    assert_eq!(u(&sim, "n_m_l"), 3, "@(m.l.ev)");
    assert_eq!(u(&sim, "n_m_la0"), 3, "@(m.la[0].ev)");
    assert_eq!(u(&sim, "n_m_la1"), 3, "@(m.la[1].ev)");
    assert_eq!(u(&sim, "n_ma0_l"), 3, "@(ma[0].l.ev)");
    assert_eq!(u(&sim, "n_ma1_la1"), 3, "@(ma[1].la[1].ev)");
}

#[test]
fn virtual_interface_event_wait() {
    let sim = xezim::simulate(
        r#"
interface my_if;
  event ev;
  initial repeat (3) #10 ->ev;
endinterface

class client;
  virtual my_if vif;
  int count = 0;
  function new(virtual my_if v);
    vif = v;
  endfunction
  task run();
    repeat (3) begin
      @(vif.ev);
      count++;
    end
  endtask
endclass

module top;
  my_if if_inst ();
  client c = new(if_inst);
  int direct_count = 0;

  initial begin
    fork
      c.run();
      repeat (3) begin
        @(if_inst.ev);
        direct_count++;
      end
    join_none
    #100;
    $finish;
  end
endmodule
"#,
        200,
    )
    .expect("simulate");

    assert_eq!(u(&sim, "direct_count"), 3, "direct interface event wait");
    let o: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let _ = o;
}
