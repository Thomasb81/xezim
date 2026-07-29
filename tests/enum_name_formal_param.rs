// Self-test: enum `.name()` reflection when called on formal method parameters.
// Ensures that `m.name()` inside a function/task correctly resolves `m`'s
// enum typedef rather than falling back to an incorrect enum table.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(&format!("top.{}", n))
        .or_else(|| sim.get_signal(n))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

const SRC: &str = r#"
module top;
  typedef enum { VAL_A = 0, VAL_B = 1 } mode_e;

  int pass = 1;

  function automatic string get_mode_name(mode_e m);
    return m.name();
  endfunction

  initial begin
    mode_e m1 = VAL_A;
    mode_e m2 = VAL_B;
    string s1, s2;

    s1 = get_mode_name(m1);
    s2 = get_mode_name(m2);

    if (s1 != "VAL_A" || s2 != "VAL_B") begin
      pass = 0;
    end
  end
endmodule
"#;

#[test]
fn test_enum_name_formal_param() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "pass"),
        1,
        "Enum .name() on formal parameter must return correct enum member name"
    );
}
