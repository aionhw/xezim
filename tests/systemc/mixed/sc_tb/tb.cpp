// sc_main testbench for the SystemVerilog module `shifter`, declared to
// SystemC as a foreign module (its ports by name).
#include <systemc.h>
class shifter : public sc_foreign_module {
public:
    sc_in<bool> clk, load;
    sc_in<sc_uint<8> > din;
    sc_in<bool> sin;
    sc_out<sc_uint<8> > q;
    shifter(sc_module_name nm)
        : sc_foreign_module(nm, "shifter"), clk("clk"), load("load"), din("din"), sin("sin"), q("q") {}
};
SC_MODULE(drv) {
    sc_in<bool> clk;
    sc_out<bool> load, sin;
    sc_out<sc_uint<8> > din;
    sc_in<sc_uint<8> > q;
    void run() {
        const char* bits = "1011001";
        din.write(0xA5);
        load.write(true);
        wait(clk.negedge_event());
        load.write(false);
        std::cout << sc_time_stamp() << " loaded " << std::hex << q.read() << std::dec << std::endl;
        for (int i = 0; bits[i] != 0; i++) {
            sin.write(bits[i] == '1');
            wait(clk.negedge_event());
            std::cout << sc_time_stamp() << " sin=" << bits[i] << " q=" << std::hex << q.read() << std::dec << std::endl;
        }
        sc_stop();
    }
    SC_CTOR(drv) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<bool> load, sin;
    sc_signal<sc_uint<8> > din, q;
    shifter s("s");
    drv d("d");
    s.clk(clk); s.load(load); s.din(din); s.sin(sin); s.q(q);
    d.clk(clk); d.load(load); d.sin(sin); d.din(din); d.q(q);
    sc_start();
    return 0;
}
