// §35.5.4 DPI export: C (an imported context function) calls back into
// exported SystemVerilog functions and a task.
module tb;
  export "DPI-C" function sv_scale;
  export "DPI-C" function sv_combine;
  export "DPI-C" task     sv_record;

  int recorded = -1;
  function int sv_scale(int x);          return x * 3;   endfunction
  function int sv_combine(int a, int b); return a + b;   endfunction
  task sv_record(int v);                 recorded = v;   endtask

  import "DPI-C" context function int c_roundtrip(input int x);

  int failures = 0;
  int r;
  initial begin
    r = c_roundtrip(10);   // sv_scale(10)=30; sv_combine(30,10)=40; record 40; return 41
    if (r != 41)        begin failures++; $display("FAIL: return %0d != 41", r); end
    if (recorded != 40) begin failures++; $display("FAIL: recorded %0d != 40", recorded); end
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    $finish;
  end
endmodule
