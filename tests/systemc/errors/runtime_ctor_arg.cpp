#include <systemc.h>
SC_MODULE(m) {
    int size;
    m(sc_module_name n, int s) : sc_module(n), size(s) {}
};
int sc_main(int argc, char* argv[]) {
    int size = 10;
    if (argc > 1)
        size = atoi(argv[1]);
    m x("x", size);
    sc_start(1, SC_NS);
    return 0;
}
