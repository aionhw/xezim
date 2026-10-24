// sc_buffer notifies on every write; sc_signal only on a value change.
#include <systemc.h>
SC_MODULE(w) {
    sc_signal<int> s;
    sc_buffer<int> b;
    int ns, nb;
    void writer() {
        // The signal and buffer writes are half a step apart: the order of
        // processes woken in the same delta cycle is unspecified (§4.2.1.2).
        for (int i = 0; i < 4; i++) {
            s.write(7);
            wait(500, SC_PS);
            b.write(7);
            wait(500, SC_PS);
        }
        s.write(8);
        wait(500, SC_PS);
        b.write(8);
        wait(1, SC_NS);
        std::cout << "signal events " << ns << " buffer events " << nb << std::endl;
    }
    void on_s() { ns++; std::cout << sc_time_stamp() << " s=" << s.read() << std::endl; }
    void on_b() { nb++; std::cout << sc_time_stamp() << " b=" << b.read() << std::endl; }
    SC_CTOR(w) : ns(0), nb(0) {
        SC_THREAD(writer);
        SC_METHOD(on_s);
        sensitive << s;
        dont_initialize();
        SC_METHOD(on_b);
        sensitive << b;
        dont_initialize();
    }
};
int sc_main(int, char*[]) {
    w x("x");
    sc_start(10, SC_NS);
    return 0;
}
