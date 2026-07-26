# Update Notes

- Merged the forward-referenced parameter sizing fix and its regression test.
- Preserved constant multidimensional array dependency tracking and added
  regression coverage.
- Reconstructed the reported DRAM sequence from `dump.vcd`: a 256-cycle AXI
  burst, final `{7,6,5,4}` word, one AXI edge, and a 128-edge DRAM wait.
- Confirmed the reconstructed sequence resumes at normalized time 7730
  (original VCD time 163930) and begins driving `ddrc_dram_resp_vld`.
- Added `remaining_events` to progress and hang reports so counted event waits
  can be distinguished from dead clocks or repeatedly reconstructed waiters.

Verification:

```text
cargo test --test sequential_event_waits
7 passed; 0 failed

cargo test --test dead_clock_watchdog
2 passed; 0 failed
```
