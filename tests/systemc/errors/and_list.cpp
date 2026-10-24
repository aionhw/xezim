#include <systemc.h>
SC_MODULE(m) {
    sc_event a, b;
    void run() {
        wait(a & b);
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m x("x");
    sc_start(1, SC_NS);
    return 0;
}
