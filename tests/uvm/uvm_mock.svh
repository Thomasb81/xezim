// Simple mock of UVM for testing purposes

package uvm_pkg;
  typedef enum { UVM_NONE, UVM_LOW, UVM_MEDIUM, UVM_HIGH, UVM_FULL, UVM_DEBUG } uvm_verbosity;

  virtual class uvm_void;
  endclass

  virtual class uvm_object extends uvm_void;
    string name;
    function new(string name = "");
      this.name = name;
    endfunction
    virtual function string get_name();
      return name;
    endfunction
  endclass

  virtual class uvm_report_object extends uvm_object;
    function new(string name = "");
      super.new(name);
    endfunction
    function void uvm_report_info(string id, string message, int verbosity = UVM_MEDIUM, string file = "", int line = 0);
      $display("[%t] UVM_INFO %s: %s", $time, id, message);
    endfunction
  endclass

  virtual class uvm_component extends uvm_report_object;
    uvm_component parent;
    function new(string name = "", uvm_component parent = null);
      super.new(name);
      this.parent = parent;
    endfunction
    
    virtual task run_phase(uvm_phase phase);
    endtask
  endclass

  class uvm_phase;
    string name;
    function new(string name = "");
      this.name = name;
    endfunction
    function void raise_objection(uvm_object obj);
    endfunction
    function void drop_objection(uvm_object obj);
    endfunction
  endclass

  // Needed so uses_real_uvm() returns true when PURE_SV_LRM=0.
  class uvm_objection;
  endclass

  // TLM analysis port: connect() and write() are intercepted by xezim's
  // tlm_deliver shim when uses_real_uvm() is true (PURE_SV_LRM=0).
  class uvm_analysis_port #(type T = int);
    string name;
    function new(string n, uvm_component parent = null); name = n; endfunction
    function void connect(/* imp */ imp); endfunction
    function void write(T t); endfunction
  endclass

  // Plain analysis imp (no suffix): write() forwards to m_imp.write().
  class uvm_analysis_imp #(type T = int, type IMP = int);
    string name;
    IMP m_imp;
    function new(string n, IMP imp); name = n; m_imp = imp; endfunction
    function void write(T t); m_imp.write(t); endfunction
  endclass

  // uvm_analysis_imp_decl(_in) / (_out): write() dispatches to the suffixed
  // method on the subscriber so xezim's tlm_deliver can call imp.write()
  // and still reach write_in() / write_out() on the implementer.
  class uvm_analysis_imp_in #(type T = int, type IMP = int);
    string name;
    IMP m_imp;
    function new(string n, IMP imp); name = n; m_imp = imp; endfunction
    function void write(T t); m_imp.write_in(t); endfunction
  endclass

  class uvm_analysis_imp_out #(type T = int, type IMP = int);
    string name;
    IMP m_imp;
    function new(string n, IMP imp); name = n; m_imp = imp; endfunction
    function void write(T t); m_imp.write_out(t); endfunction
  endclass

  class uvm_root extends uvm_component;
    uvm_component test_inst;

    function new(string name = "", uvm_component parent = null);
      super.new(name, parent);
    endfunction

    task run_test_internal(string test_name = "");
      uvm_phase phase = new("run");
      $display("UVM_INFO: Running test %s", test_name);
      if (test_inst != null) begin
        test_inst.run_phase(phase);
      end
    endtask
  endclass

endpackage

`define uvm_info(ID, MSG, VERBOSITY) \
   begin \
     $display("UVM_INFO %s: %s", ID, MSG); \
   end

`define uvm_component_utils(TYPE) \
   // simplified

