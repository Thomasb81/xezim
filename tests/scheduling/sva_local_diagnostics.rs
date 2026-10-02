//! §16.10: unsupported property/sequence local variables must produce a
//! diagnostic instead of silently dropping the assertion body.

#[test]
fn assertion_local_state_is_rejected_explicitly() {
    for kind in ["property", "sequence"] {
        let source = format!(
            "module tb; bit clk; {kind} duration; time stamp; \
             @(posedge clk) (1'b1, stamp = $time) ##1 ($time > stamp); \
             end{kind} ap: assert property (duration); endmodule"
        );
        match xezim::simulate(&source, 10) {
            Ok(_) => panic!("{kind}-local state was silently accepted"),
            Err(error) => assert!(
                error.contains(&format!(
                    "{kind}-local variable declarations are not supported"
                )),
                "wrong diagnostic: {error}"
            ),
        }
    }
}
