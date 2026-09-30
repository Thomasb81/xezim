//! `@(arr[i])` waits for a change of one unpacked-array element.
//!
//! On a small array it works. On an array large enough to be stored without
//! per-element names (here 200,000 entries) the event control is dropped with a
//! warning, for a constant index and a variable index alike, so the waiting
//! process never wakes. The `#[ignore]`d test pins that divergence until it is
//! fixed. Expected values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn small_array_element_event_wakes() {
    let o = out(r#"
module top;
  logic [7:0] sm [0:3];
  int hits = 0;
  initial forever @(sm[2]) begin hits++; $display("T|sm t=%0t", $time); end
  initial begin
    #1 sm[1] = 8'h11;
    #1 sm[2] = 8'h44;
    #1 $display("T|hits %0d", hits);
    $finish;
  end
endmodule
"#);
    assert_eq!(o, ["T|sm t=2", "T|hits 1"], "{o:?}");
}

#[test]
#[ignore = "known gap: @(big[i]) on a large unnamed array is dropped"]
fn large_array_element_events_wake() {
    let o = out(r#"
module top;
  logic [7:0] big [0:199999];
  int i = 7;
  int hits_c = 0, hits_v = 0;
  initial forever @(big[5]) begin hits_c++; $display("T|const t=%0t v=%h", $time, big[5]); end
  initial forever @(big[i]) begin hits_v++; $display("T|var t=%0t v=%h", $time, big[i]); end
  initial begin
    #1 big[5] = 8'h11;
    #1 big[7] = 8'h22;
    #1 big[6] = 8'h33;
    #1 $display("T|hits %0d %0d", hits_c, hits_v);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        o,
        ["T|const t=1 v=11", "T|var t=2 v=22", "T|hits 1 1"],
        "{o:?}"
    );
}
