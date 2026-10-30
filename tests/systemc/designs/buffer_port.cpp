// §6.9 an sc_buffer notifies on every write, also through the ports bound
// to it (§6.4: a port's value_changed_event is its channel's).
#include <systemc.h>
SC_MODULE(m) {
    sc_in<int> in;
    int n;
    void on() { n++; std::cout << sc_time_stamp() << " in=" << in.read() << " #" << n << std::endl; }
    SC_CTOR(m) : n(0) { SC_METHOD(on); sensitive << in; dont_initialize(); }
};
SC_MODULE(src) {
    sc_out<int> out;
    void run() {
        for (int i = 0; i < 3; i++) { out.write(5); wait(1, SC_NS); }
        out.write(6);
    }
    SC_CTOR(src) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_buffer<int> b;
    m x("x");
    src s("s");
    x.in(b);
    s.out(b);
    sc_start(10, SC_NS);
    return 0;
}
