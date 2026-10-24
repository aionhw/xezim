// A SystemC counter and monitor instantiated by the SystemVerilog
// testbench tb.sv (no sc_main: the processes start at time 0).
#include <systemc.h>
SC_MODULE(counter) {
    sc_in<bool> clk, rst, en;
    sc_out<sc_uint<4> > q;
    void tick() {
        if (rst.read()) q.write(0);
        else if (en.read()) q.write(q.read() + 1);
    }
    void show() {
        std::cout << sc_time_stamp() << " q=" << q.read() << std::endl;
    }
    SC_CTOR(counter) {
        SC_METHOD(tick);
        sensitive << clk.pos();
        dont_initialize();
        SC_METHOD(show);
        sensitive << q;
        dont_initialize();
    }
};
