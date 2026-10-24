#include <systemc.h>
SC_MODULE(m) {
    void run() {
        try {
            wait(1, SC_NS);
        } catch (...) {
        }
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m x("x");
    sc_start(2, SC_NS);
    return 0;
}
