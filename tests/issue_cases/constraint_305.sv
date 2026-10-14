// Issue #305: `std::randomize(delays)` inside a child instance must target
// the CHILD's own dynamic array, not the parent's same-named array. The
// solver and its inline checker previously resolved the bare receiver name
// (`delays`) to the parent scope's storage, so the parent's array got
// randomized (and its `.size()` read as 0 by the checker), rejecting a valid
// solution and leaving the child's array empty.
// Expected: CHILD is sized to 16 and filled with in-window values; TOP's
// array stays empty; randomize returns 1 (no spurious failure).
module child_mod ;
   int        period ;
   bit [15:0] delays [] ;

   function automatic void in_window(bit [15:0] v, int lo, int hi, inout int bad);
      if (v < lo || v > hi) begin
         bad++ ;
         $display("out-of-range: delays[%0d]=%0d", 0, v) ;
      end
   endfunction

   initial begin
      int bad = 0 ;
      period = 1000 ;
      if (!std::randomize(delays) with {
         delays.size() == 16 ;
         foreach (delays[i]) {
            delays[i] >= 0.9 * period ;
            delays[i] <= 1.1 * period ;
         }
      }) begin
         $display("CHILD: spurious randomize failure");
         $fatal(1) ;
      end
      in_window(delays[0], 900, 1100, bad) ;
      if (delays.size() != 16) begin
         $display("CHILD: wrong size %0d", delays.size());
         $fatal(1) ;
      end
      if (bad > 0) $fatal(1) ;
      if (delays.size() == 16 && delays[0] >= 900 && delays[0] <= 1100)
         $display("TEST_PASS");
      else begin
         $display("TEST_FAIL");
         $fatal(1);
      end
   end
endmodule

module top_mod ;
   bit [15:0] delays [] ;
   child_mod  u1 () ;
   initial begin
      #1 ;
      if (delays.size() != 0) begin
         $display("TOP: leaked randomization into parent (size=%0d)", delays.size());
         $fatal(1) ;
      end
   end
endmodule