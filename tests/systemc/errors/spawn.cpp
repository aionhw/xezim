#define SC_INCLUDE_DYNAMIC_PROCESSES
#include <systemc.h>
SC_MODULE(m) {
    void run() {
        sc_spawn([this]() { wait(1, SC_NS); });
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m x("x");
    sc_start(1, SC_NS);
    return 0;
}
