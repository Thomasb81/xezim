// Test for method call chaining - internal method calls should execute properly
module top;

  class test_server;
    int call_counter = 0;
    int data = 0;
    
    // Internal method that tracks calls
    function void track_call(int val);
      call_counter++;
      data = val;
      $display("[internal] track_call: counter=%0d data=%0d", call_counter, data);
    endfunction
    
    // Method that calls internal method
    function void outer_method();
      $display("[outer] before internal");
      this.track_call(42);
      $display("[outer] after internal");
    endfunction
    
    // Another internal method
    function void process();
      call_counter++;
      $display("[internal] process: counter=%0d", call_counter);
    endfunction
    
    // Yet another
    function void finish();
      call_counter++;
      $display("[internal] finish: counter=%0d", call_counter);
    endfunction
  endclass

  test_server ts;

  initial begin
    ts = new();
    
    $display("=== Starting method chain test ===");
    
    // Call method that calls internal method
    ts.outer_method();
    
    // Direct calls to internal methods
    ts.process();
    ts.finish();
    
    $display("=== Final counter: %0d ===", ts.call_counter);
    
    if (ts.call_counter == 3) begin
      $display("TAG_PASS");
    end else begin
      $display("TAG_FAIL expected 3 got %0d", ts.call_counter);
    end
  end

endmodule
