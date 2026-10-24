// Four-valued sc_logic/sc_lv: X and Z values, resolution-free assignment,
// printing, and conversion of known values.
#include <systemc.h>
SC_MODULE(lg) {
    sc_signal<sc_logic> l;
    sc_signal<sc_lv<4> > v;
    void drive() {
        l.write(SC_LOGIC_X);
        v.write("01XZ");
        wait(1, SC_NS);
        l.write(SC_LOGIC_1);
        v.write("1010");
        wait(1, SC_NS);
        l.write(SC_LOGIC_Z);
        v.write("ZZ11");
        wait(1, SC_NS);
    }
    void show() {
        std::cout << sc_time_stamp() << " l=" << l.read() << " v=" << v.read() << std::endl;
    }
    SC_CTOR(lg) {
        SC_THREAD(drive);
        SC_METHOD(show);
        sensitive << l << v;
        dont_initialize();
    }
};
int sc_main(int, char*[]) {
    lg x("x");
    sc_start(5, SC_NS);
    sc_lv<4> k = "1100";
    std::cout << "k=" << k << " uint=" << k.to_uint() << std::endl;
    return 0;
}
