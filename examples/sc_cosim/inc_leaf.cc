// Mode B leaf: a minimal pure-SystemC module. Under an SV master it is
// instantiated directly in the SV netlist (see sv_leaf_top.sv). The SC module
// knows nothing about xezim aside from including sc_api.h for the shim ABI;
// its ports are written as plain sc_in/sc_out members, which xezim reflects
// to synthesize the SV placeholder and auto-bind the crossing.
#include <systemc.h>
#include <cstdint>
#include "sc_api.h"

SC_MODULE(inc_leaf)
{
public:
    sc_in<bool> clock;
    sc_in<sc_uint<8> > din;
    sc_out<sc_uint<8> > dout;
    sc_uint<8> reg_;

    SC_CTOR(inc_leaf)
        : clock("clock"), din("din"), dout("dout"), reg_(0)
    {
        SC_METHOD(sync);
        sensitive << clock.pos();
        dont_initialize();
    }

    void sync()
    {
        // Registered copy of the input plus one, re-emitted each posedge.
        reg_ = (sc_uint<8>)(din.read() + 1);
        dout.write(reg_);
    }
};