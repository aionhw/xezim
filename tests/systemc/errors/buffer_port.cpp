#include <systemc.h>
SC_MODULE(m) {
    sc_in<int> in;
    SC_CTOR(m) {}
};
int sc_main(int, char*[]) {
    sc_buffer<int> b;
    m x("x");
    x.in(b);
    sc_start(1, SC_NS);
    return 0;
}
