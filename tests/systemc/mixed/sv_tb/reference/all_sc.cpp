// All-SystemC equivalent of counter.cpp + tb.sv (its output is the golden).
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
SC_MODULE(tb) {
    sc_in<bool> clk;
    sc_out<bool> rst, en;
    sc_in<sc_uint<4> > q;
    void run() {
        rst.write(true);
        wait(clk.negedge_event());
        wait(clk.negedge_event());
        rst.write(false);
        en.write(true);
        for (int i = 0; i < 18; i++) wait(clk.negedge_event());
        en.write(false);
        for (int i = 0; i < 2; i++) wait(clk.negedge_event());
        std::cout << "tb: final q=" << q.read() << std::endl;
        sc_stop();
    }
    SC_CTOR(tb) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS, 0.5, 5, SC_NS, true);
    sc_signal<bool> rst, en;
    sc_signal<sc_uint<4> > q;
    counter u("u");
    tb t("t");
    u.clk(clk); u.rst(rst); u.en(en); u.q(q);
    t.clk(clk); t.rst(rst); t.en(en); t.q(q);
    sc_start();
    return 0;
}
