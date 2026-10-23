// t11: SC_CTHREAD with async reset, wait(n), and a reset pulse mid-run
#include <systemc.h>
SC_MODULE(cnt) {
    sc_in<bool> clk, rst_n;
    sc_out<int> q;
    void run() {
        int v = 0;
        q.write(v);
        std::cout << sc_time_stamp() << " reset section" << std::endl;
        wait();
        while (true) {
            v++;
            q.write(v);
            wait(2);
        }
    }
    SC_CTOR(cnt) { SC_CTHREAD(run, clk.pos()); async_reset_signal_is(rst_n, false); }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<bool> rst_n;
    sc_signal<int> q;
    cnt c("c"); c.clk(clk); c.rst_n(rst_n); c.q(q);
    rst_n = false;
    sc_start(15, SC_NS);
    rst_n = true;
    sc_start(80, SC_NS);
    std::cout << sc_time_stamp() << " q=" << q.read() << std::endl;
    rst_n = false;
    sc_start(3, SC_NS);
    std::cout << sc_time_stamp() << " q=" << q.read() << std::endl;
    rst_n = true;
    sc_start(40, SC_NS);
    std::cout << sc_time_stamp() << " q=" << q.read() << std::endl;
    return 0;
}
