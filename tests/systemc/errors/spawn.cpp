#include <systemc.h>
SC_MODULE(m) {
    void child() {}
    void run() {
        sc_spawn(sc_bind(&m::child, this));
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m x("x");
    sc_start(1, SC_NS);
    return 0;
}
