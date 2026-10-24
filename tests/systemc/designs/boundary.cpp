// sc_start(t) in steps: events exactly at the end of a step run in the next
// sc_start; sc_main writes apply at the next sc_start.
#include <systemc.h>
SC_MODULE(b) {
    sc_in<int> in;
    sc_event e;
    void timer() {
        for (int i = 0; i < 3; i++) {
            wait(5, SC_NS);
            std::cout << sc_time_stamp() << " timer " << i << std::endl;
        }
    }
    void watch() { std::cout << sc_time_stamp() << " in=" << in.read() << std::endl; }
    SC_CTOR(b) {
        SC_THREAD(timer);
        SC_METHOD(watch);
        sensitive << in;
    }
};
int sc_main(int, char*[]) {
    sc_signal<int> s;
    b x("x");
    x.in(s);
    s.write(1);
    sc_start(5, SC_NS);
    std::cout << "main " << sc_time_stamp() << std::endl;
    s.write(2);
    sc_start(5, SC_NS);
    std::cout << "main " << sc_time_stamp() << std::endl;
    s.write(3);
    s.write(4);
    sc_start(10, SC_NS);
    std::cout << "main " << sc_time_stamp() << std::endl;
    return 0;
}
