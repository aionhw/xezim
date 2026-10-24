// SystemC accumulator between two SystemVerilog levels: instantiated by
// top.sv, it uses the SystemVerilog module `addsat` (leaf.sv) through an
// sc_foreign_module whose SV name comes from hdl_name().
#include <systemc.h>
class addsat_fm : public sc_foreign_module {
public:
    sc_in<int> a, b;
    sc_out<int> y;
    addsat_fm(sc_module_name nm) : sc_foreign_module(nm), a("a"), b("b"), y("y") {}
    const char* hdl_name() const { return "addsat"; }
};
SC_MODULE(mac) {
    sc_in<bool> clk, clr;
    sc_in<int> x;
    sc_out<int> acc;
    sc_signal<int> sum;
    addsat_fm add;
    void reg() {
        int v = clr.read() ? 0 : sum.read();
        acc.write(v);
        std::cout << sc_time_stamp() << " acc=" << v << std::endl;
    }
    SC_CTOR(mac) : add("add") {
        add.a(acc);
        add.b(x);
        add.y(sum);
        SC_METHOD(reg);
        sensitive << clk.pos();
        dont_initialize();
    }
};
