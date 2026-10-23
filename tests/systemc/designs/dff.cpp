// t1: D flip-flop with async reset (SC_METHOD, edge sensitivity)
#include <systemc.h>

SC_MODULE(dff) {
    sc_in<bool> clk, rst_n, d;
    sc_out<bool> q;
    void run() {
        if (!rst_n.read()) q.write(false);
        else if (clk.posedge()) q.write(d.read());
    }
    SC_CTOR(dff) {
        SC_METHOD(run);
        sensitive << clk.pos() << rst_n.neg();
    }
};

int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<bool> rst_n, d, q;
    dff u("u");
    u.clk(clk); u.rst_n(rst_n); u.d(d); u.q(q);
    rst_n = false; d = true;
    sc_start(12, SC_NS);
    std::cout << "after reset q=" << q.read() << std::endl;
    rst_n = true;
    sc_start(10, SC_NS);
    std::cout << "after edge  q=" << q.read() << std::endl;
    d = false;
    sc_start(10, SC_NS);
    std::cout << "after edge  q=" << q.read() << std::endl;
    return 0;
}
