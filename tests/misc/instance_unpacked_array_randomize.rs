//! Regression for issue #303: a fixed-size unpacked array declared in an
//! INSTANTIATED (inlined) module was silently skipped by `std::randomize`.
//!
//! The array's geometry is registered in `module.arrays` under the
//! scope-qualified key (`u_child.delays`), but its base name is NOT a scalar
//! signal — only the element signals are. `resolve_hier_name` of the bare
//! `delays` inside the child instance therefore fell through to the
//! unprefixed global key (which has no geometry) and `array_operand_name`
//! reported the operand unknown, so `std::randomize(delays)` solved nothing
//! and left every element at its default 0 with no warning. The instance-
//! scoped name resolution now consults the collection tables too, so a bare
//! array name binds to `<scope>.<name>` and randomize reaches all elements.
use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A fixed-size unpacked array in an instantiated module must be randomized
/// (each element inside the constrained [900, 1100] band) rather than left at
/// 0 with no warning.
#[test]
fn instance_fixed_unpacked_array_is_randomized() {
    const SRC: &str = r#"
module child_mod;
	int period;
	bit [15:0] delays [16];
	initial begin
		period = 1000;
		void'(std::randomize(delays) with {
			foreach (delays[i]) {
				delays[i] >= 0.9 * period;
				delays[i] <= 1.1 * period;
			}
		});
		if (delays[0] >= 900 && delays[0] <= 1100)
			$display("MWE-FIXED: PASS first=%0d", delays[0]);
		else
			$display("MWE-FIXED: FAIL first=%0d", delays[0]);
		$finish;
	end
endmodule

module top_mod;
	child_mod u_child();
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let msg = line(&sim, "MWE-FIXED:");
    // The constrained element must land in [900, 1100]; the bug left it at 0.
    assert!(
        msg.starts_with("MWE-FIXED: PASS"),
        "array element must be randomized into [900,1100], got {msg:?}"
    );
}