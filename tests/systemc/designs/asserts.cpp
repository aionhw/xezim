// sc_assert (SystemC's fatal error F4) and assert (<cassert>): passing
// checks do nothing; a failing sc_assert in a process reports where, and
// aborts the run (exit status 134).
#include <systemc.h>
#include <cassert>

SC_MODULE(m) {
    int n;
    void run() {
        while (true) {
            wait(2, SC_NS);
            assert(n >= 0);
            sc_assert(n < 10);
            std::cout << sc_time_stamp() << " n " << n << std::endl;
            n += 4;
            sc_assert(n < 10);
        }
    }
    SC_CTOR(m) : n(0) { SC_THREAD(run); }
};

int sc_main(int argc, char* argv[]) {
    sc_assert(argc == 1);
    m u("u");
    sc_start(20, SC_NS);
    std::cout << "not reached" << std::endl;
    return 0;
}
