// sc_clock options: duty cycle, start time, posedge_first = false;
// negedge sensitivity and sc_time printing at several magnitudes.
#include <systemc.h>
SC_MODULE(mon) {
    sc_in<bool> a, b;
    void on_a() { std::cout << sc_time_stamp() << " a=" << a.read() << std::endl; }
    void on_b_neg() { std::cout << sc_time_stamp() << " b negedge" << std::endl; }
    SC_CTOR(mon) {
        SC_METHOD(on_a);
        sensitive << a;
        dont_initialize();
        SC_METHOD(on_b_neg);
        sensitive << b.neg();
        dont_initialize();
    }
};
int sc_main(int, char*[]) {
    sc_clock ca("ca", 10, SC_NS, 0.3, 5, SC_NS, true);
    sc_clock cb("cb", sc_time(1, SC_US), 0.5, sc_time(0, SC_NS), false);
    mon m("m");
    m.a(ca);
    m.b(cb);
    sc_start(42, SC_NS);
    std::cout << "now " << sc_time_stamp() << std::endl;
    sc_start(2, SC_US);
    std::cout << "now " << sc_time_stamp() << std::endl;
    return 0;
}
