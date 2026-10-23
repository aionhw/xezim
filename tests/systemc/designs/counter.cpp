// t2: 4-bit counter as SC_CTHREAD with synchronous reset, sc_uint arithmetic
#include <systemc.h>

SC_MODULE(counter) {
    sc_in<bool> clk, rst;
    sc_out<sc_uint<4> > count;
    void run() {
        sc_uint<4> v = 0;
        count.write(v);
        wait();
        while (true) {
            v = v + 1;
            count.write(v);
            wait();
        }
    }
    SC_CTOR(counter) {
        SC_CTHREAD(run, clk.pos());
        reset_signal_is(rst, true);
    }
};

int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<bool> rst;
    sc_signal<sc_uint<4> > count;
    counter c("c");
    c.clk(clk); c.rst(rst); c.count(count);
    rst = true;
    sc_start(20, SC_NS);
    rst = false;
    for (int i = 0; i < 18; ++i) sc_start(10, SC_NS);
    std::cout << "count=" << count.read() << " (wraps at 16)" << std::endl;
    return 0;
}
