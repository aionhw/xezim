// All-SystemC equivalent of top.sv + mac.cpp + leaf.sv (its output is the
// golden).
#include <systemc.h>
SC_MODULE(addsat) {
    sc_in<int> a, b;
    sc_out<int> y;
    void eval() {
        long long s = (long long)a.read() + (long long)b.read();
        if (s > 2147483647LL) y.write(2147483647);
        else if (s < -2147483648LL) y.write(-2147483647 - 1);
        else y.write((int)s);
    }
    SC_CTOR(addsat) {
        SC_METHOD(eval);
        sensitive << a << b;
    }
};
SC_MODULE(mac) {
    sc_in<bool> clk, clr;
    sc_in<int> x;
    sc_out<int> acc;
    sc_signal<int> sum;
    addsat add;
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
SC_MODULE(top) {
    sc_in<bool> clk;
    sc_out<bool> clr;
    sc_out<int> x;
    sc_in<int> acc;
    void run() {
        int samples[8] = {3, 100, 2000, 30000, 500000, 2147000000, -7, 1};
        clr.write(true);
        wait(clk.negedge_event());
        clr.write(false);
        for (int i = 0; i < 8; i++) {
            x.write(samples[i]);
            wait(clk.negedge_event());
        }
        std::cout << "top: acc=" << acc.read() << std::endl;
        sc_stop();
    }
    SC_CTOR(top) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS, 0.5, 5, SC_NS, true);
    sc_signal<bool> clr;
    sc_signal<int> x, acc;
    mac m("m");
    top t("t");
    m.clk(clk); m.clr(clr); m.x(x); m.acc(acc);
    t.clk(clk); t.clr(clr); t.x(x); t.acc(acc);
    sc_start();
    return 0;
}
