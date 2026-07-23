// Regression for uvm_analysis_imp_decl write routing.
//
// tlm_deliver previously shortcut directly to implementer.write(), which is a
// no-op for imp_decl subscribers (their write() forwards to write_in/write_out).
// Fix: tlm_deliver calls imp.write() so the imp's own SV method dispatches to
// the correct suffixed method on the implementer.
//
// Run with PURE_SV_LRM=0 so uses_real_uvm() is true and TLM interception
// is active.
`include "uvm_mock.svh"

module top;
  import uvm_pkg::*;

  class scoreboard;
    int n_in, n_out;
    function void write_in(int t);  n_in++;  endfunction
    function void write_out(int t); n_out++; endfunction
  endclass

  initial begin
    automatic scoreboard scb = new;
    automatic uvm_analysis_imp_in  #(int, scoreboard) imp_in  = new("imp_in",  scb);
    automatic uvm_analysis_imp_out #(int, scoreboard) imp_out = new("imp_out", scb);
    automatic uvm_analysis_port #(int) in_ap  = new("in_ap");
    automatic uvm_analysis_port #(int) out_ap = new("out_ap");

    in_ap.connect(imp_in);
    out_ap.connect(imp_out);

    in_ap.write(7);
    in_ap.write(8);
    out_ap.write(9);

    if (scb.n_in !== 2 || scb.n_out !== 1)
      $display("TEST_FAIL: n_in=%0d (exp 2) n_out=%0d (exp 1)", scb.n_in, scb.n_out);
    else
      $display("TEST_PASS");
  end
endmodule
