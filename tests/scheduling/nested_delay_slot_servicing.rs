//! IEEE 1800-2017 §4.4.2.4: the postponed region (`$monitor`, `$strobe`) and
//! the waveform dump belong to EVERY time slot, including slots consumed by a
//! nested event loop.
//!
//! A `#delay` inside an edge block (`always @(posedge tick) begin #500; end`)
//! runs through `exec_statement`, which advances time with its own nested loop
//! (`run_events_until`). That loop ran processes, applied NBAs, settled and
//! checked edges for each slot it crossed — but ran none of the postponed
//! region for them. Anything that changed in one of those slots was invisible
//! to `$monitor` and to the dump until some LATER slot that happened to be
//! serviced, and because the two are serviced by different mechanisms they
//! disagreed with each other as well as with the change itself.
//!
//! Observed on a real design: a reset released at t=100 was reported by
//! `$monitor` at 102 and by the VCD at 112, while an internal trace of the
//! write itself said 100. The dump was the worse of the two — it held no
//! record for the timestamp at all, so 485 changes were re-attributed to a
//! later one. Timing read off such a wave is simply wrong.
//!
//! Here `sig` changes at t=100, inside the window of a `#500` in an edge
//! block. Both views must say 100.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ps
module top;
  logic tick = 0;
  logic sig  = 0;

  always #50 tick = ~tick;          // first posedge at t=50

  // The nested-loop path: a delay inside an edge block.
  always @(posedge tick) begin
    #500;
  end

  initial #100 sig = 1;             // real change, inside that window
  initial #600 sig = 0;

  initial $monitor("MON %0t sig=%b", $time, sig);
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #900 $finish;
  end
endmodule
"#;

fn run(tag: &str) -> (Vec<String>, Vec<u64>) {
    // Tests in a group share one process, so the path must be per-test or the
    // three runs clobber each other's dump.
    let mut path = std::env::temp_dir();
    path.push(format!("xezim_slot_service_{}_{}.vcd", tag, std::process::id()));
    let _ = std::fs::remove_file(&path);

    let src = SRC.replace("@VCD@", path.to_str().unwrap());
    let sim = simulate(&src, 100_000_000).expect("simulate failed");

    let mons: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("MON "))
        .collect();

    let text = std::fs::read_to_string(&path).expect("VCD not written");
    let stamps: Vec<u64> = text
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let _ = std::fs::remove_file(&path);
    (mons, stamps)
}

/// §21.2.3 — `$monitor` reports the change in the slot it happened in, not in
/// whichever later slot the nested loop finally returned to (550 ns here).
#[test]
fn monitor_sees_a_change_made_inside_a_nested_delay_window() {
    let (mons, _) = run("monitor");
    assert!(
        mons.contains(&"MON 100000 sig=1".to_string()),
        "expected the t=100ns change to be reported at 100000ps, got {mons:?}"
    );
}

/// §21.7 — the dump has a record for that timestamp. Previously the VCD went
/// straight from #0 to #550000 and the change was re-dated to 550 ns.
#[test]
fn dump_has_a_record_for_a_slot_inside_a_nested_delay_window() {
    let (_, stamps) = run("record");
    assert!(
        stamps.contains(&100_000),
        "VCD has no #100000 record; timestamps were {stamps:?}"
    );
}

/// The clock kept toggling throughout the delay window, so those slots must be
/// in the dump too — they were ALL missing, not just the one under test.
#[test]
fn dump_keeps_slots_crossed_by_the_nested_loop() {
    let (_, stamps) = run("slots");
    for t in [150_000u64, 200_000, 250_000, 300_000] {
        assert!(
            stamps.contains(&t),
            "VCD lost slot #{t} inside the nested delay window; got {stamps:?}"
        );
    }
}
