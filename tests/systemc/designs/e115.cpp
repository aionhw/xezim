// §6.4.4: with the default writer policy (SC_ONE_WRITER) a second process
// writing an sc_signal is an error at that write (E115).
#include <systemc.h>
SC_MODULE(top) {
    sc_signal<int> s;
    void p1() { wait(1, SC_NS); s.write(1); }
    void p2() { wait(2, SC_NS); s.write(2); }
    void mon() { std::cout << sc_time_stamp() << " s=" << s.read() << std::endl; }
    SC_CTOR(top) {
        SC_THREAD(p1); SC_THREAD(p2);
        SC_METHOD(mon); sensitive << s; dont_initialize();
    }
};
int sc_main(int, char*[]) {
    top t("t");
    sc_start(5, SC_NS);
    std::cout << "not reached" << std::endl;
    return 0;
}
