`timescale 1ns/1ns
// Pure-SystemVerilog reproduction of the clockgen 4-state `~` bug.
//
// An uninitialized 4-state `logic` is X at :0. Per LRM §6.8, ~1'bx == 1'bx,
// so `always #5 clk = ~clk;` must NEVER toggle an X-start clock (no posedge,
// no edge at all). xezim's clock-generator fast path used to treat any
// non-One bit (X/Z included) as 0 and flip it to 1, synthesising a 0->1->0
// clock out of `clkX` and firing 6 posedges. The explicit `clk0 = 0` clock
// must still toggle normally (6 posedges over #60).
//
// Run: xezim --simulate -s top tests/clockgen_x_clock_stays_x.sv
// Expect (both the reference simulator and fixed xezim): BOTH tags:
//   TAG_PASS_clock_stays_x      xezim pre-fix: TAG_FAIL_clock_active pe=6
//   TAG_PASS_explicit_toggles   both sims agree pe=6
module top;
  logic clkX;                     // 4-state, no init -> X at :0
  always #5 clkX = ~clkX;         // ~X = X -> stays X, never edges

  logic clk0 = 0;                 // explicit init -> genuine clock
  always #5 clk0 = ~clk0;         // 0->1->0, posedges at 5,15,25,35,45,55

  int unsigned peX = 0;
  int unsigned pe0 = 0;

  initial forever @(posedge clkX) peX++;
  initial forever @(posedge clk0) pe0++;

  initial begin
    $display("CLKX t=%0t clkX=%b", $time, clkX); // report start of X clock
    #60;                    // posedges on clk0 at 5,15,25,35,45,55 -> 6
    if (peX == 0)
      $display("TAG_PASS_clock_stays_x");
    else
      $display("TAG_FAIL_clock_active peX=%0d", peX);
    if (pe0 == 6)
      $display("TAG_PASS_explicit_toggles");
    else
      $display("TAG_FAIL_explicit pe0=%0d", pe0);
    $finish;
  end
endmodule