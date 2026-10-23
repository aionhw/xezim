// t9: delta cycles (signal → method → signal chains), SC_THREAD with static
// sensitivity and wait(), methods sensitive to value changes
#include <systemc.h>
SC_MODULE(chain) {
    sc_in<bool> clk;
    sc_signal<int> a, b, c;
    void m1() { b.write(a.read() + 1); }
    void m2() { c.write(b.read() * 2); }
    void mon() { std::cout << sc_time_stamp() << " a=" << a.read() << " b=" << b.read() << " c=" << c.read() << std::endl; }
    void stim() {
        int i = 0;
        while (true) {
            wait();
            a.write(++i);
            if (i == 4) break;
        }
        std::cout << sc_time_stamp() << " stim done" << std::endl;
    }
    SC_CTOR(chain) {
        SC_METHOD(m1); sensitive << a;
        SC_METHOD(m2); sensitive << b; dont_initialize();
        SC_METHOD(mon); sensitive << c; dont_initialize();
        SC_THREAD(stim); sensitive << clk.pos();
    }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    chain ch("ch");
    ch.clk(clk);
    sc_start(60, SC_NS);
    return 0;
}
