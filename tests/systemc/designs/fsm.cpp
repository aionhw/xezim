// A Moore FSM (sequence detector for 1011) with an enum state register:
// SC_METHOD next-state logic, SC_CTHREAD state register with reset, and a
// testbench thread driving a bit pattern on the falling edge.
#include <systemc.h>
enum state_t { S0, S1, S10, S101, S1011 };
SC_MODULE(det) {
    sc_in<bool> clk, rst, din;
    sc_out<bool> hit;
    sc_signal<int> state, next;
    void comb() {
        int s = state.read();
        bool d = din.read();
        int n = S0;
        switch (s) {
        case S0: n = d ? S1 : S0; break;
        case S1: n = d ? S1 : S10; break;
        case S10: n = d ? S101 : S0; break;
        case S101: n = d ? S1011 : S10; break;
        case S1011: n = d ? S1 : S10; break;
        }
        next.write(n);
        hit.write(s == S1011);
    }
    void seq() {
        state.write(S0);
        wait();
        while (true) {
            state.write(next.read());
            wait();
        }
    }
    SC_CTOR(det) {
        SC_METHOD(comb);
        sensitive << state << din;
        SC_CTHREAD(seq, clk.pos());
        reset_signal_is(rst, true);
    }
};
SC_MODULE(tb) {
    sc_in<bool> clk, hit;
    sc_out<bool> rst, din;
    void run() {
        const char* pat = "1101011011";
        rst.write(true);
        wait(clk.negedge_event());
        wait(clk.negedge_event());
        rst.write(false);
        for (int i = 0; pat[i] != 0; i++) {
            din.write(pat[i] == '1');
            wait(clk.negedge_event());
            std::cout << sc_time_stamp() << " in=" << pat[i] << " hit=" << hit.read() << std::endl;
        }
        sc_stop();
    }
    SC_CTOR(tb) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<bool> rst, din, hit;
    det d("d");
    tb t("t");
    d.clk(clk); d.rst(rst); d.din(din); d.hit(hit);
    t.clk(clk); t.hit(hit); t.rst(rst); t.din(din);
    sc_start();
    return 0;
}
