// Issue #304: the inline-constraint checker read `delays.size()` of an
// instance-scoped, PRE-SIZED dynamic array as 0, rejecting a perfectly valid
// solution. The array lives in a child instance (`u_child`) and is
// sized with `new[16]` before `std::randomize(delays)` runs, so its `.size()`
// is genuinely 16 — but the checker resolved the bare `delays` to the bare
// leaf (whose storage is keyed `u_child.delays`, not `delays`), reading a
// size of 0 from the wrong storage key and reporting randomize failure.
// Expected: randomize succeeds, the array stays sized to 16, and the sampling
// window is respected (no spurious "no solution").
module child_mod ;
   int        period ;
   bit [15:0] delays [] ;

   function automatic void in_window(bit [15:0] v, int lo, int hi, inout int bad);
      if (v < lo || v > hi) begin
         bad++ ;
         $display("out-of-range: delays[0]=%0d", v) ;
      end
   endfunction

   initial begin
      int bad = 0 ;
      period = 1000 ;
      delays = new[16] ;
      if (!std::randomize(delays) with {
         delays.size() == 16 ;
         foreach (delays[i]) {
            delays[i] >= 0.9 * period ;
            delays[i] <= 1.1 * period ;
         }
      }) begin
         $display("CHILD: spurious randomize failure (checker read .size()=0)");
         $fatal(1) ;
      end
      in_window(delays[0], 900, 1100, bad) ;
      if (bad > 0) $fatal(1) ;
      if (delays.size() != 16) begin
         $display("CHILD: wrong size %0d", delays.size());
         $fatal(1) ;
      end
      if (delays.size() == 16 && delays[0] >= 900 && delays[0] <= 1100)
         $display("TEST_PASS");
      else begin
         $display("TEST_FAIL");
         $fatal(1);
      end
   end
endmodule

module top_mod ;
   child_mod  u_child () ;
endmodule