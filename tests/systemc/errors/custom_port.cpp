#include <systemc.h>
class bus_if : virtual public sc_interface {
public:
    virtual int read(int a) = 0;
};
SC_MODULE(m) {
    sc_port<bus_if> bus;
    SC_CTOR(m) {}
};
int sc_main(int, char*[]) {
    m x("x");
    sc_start(1, SC_NS);
    return 0;
}
