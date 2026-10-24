// Constructor parameters with SC_HAS_PROCESS, an out-of-class constructor,
// child modules as members built in the initializer list, positional
// binding and sc_inout.
#include <systemc.h>
SC_MODULE(acc) {
    sc_in<bool> clk;
    sc_inout<int> total;
    int step;
    void tick() { total.write(total.read() + step); }
    SC_HAS_PROCESS(acc);
    acc(sc_module_name n, int s) : sc_module(n), step(s) {
        SC_METHOD(tick);
        sensitive << clk.pos();
        dont_initialize();
    }
};
class pair_top : public sc_module {
public:
    sc_in<bool> clk;
    sc_signal<int> t1, t2;
    acc a1, a2;
    SC_HAS_PROCESS(pair_top);
    pair_top(sc_module_name n);
    void show() {
        std::cout << sc_time_stamp() << " t1=" << t1.read() << " t2=" << t2.read() << std::endl;
    }
};
pair_top::pair_top(sc_module_name n) : sc_module(n), a1("a1", 1), a2("a2", 5) {
    a1(clk, t1);
    a2.clk(clk);
    a2.total(t2);
    SC_METHOD(show);
    sensitive << clk.neg();
    dont_initialize();
}
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    pair_top p("p");
    p.clk(clk);
    sc_start(45, SC_NS);
    return 0;
}
