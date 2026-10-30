// §4.5.7 an unbounded sc_start() returns when no events are left; sc_main
// can then change the design and call sc_start again, bounded or not.
#include <systemc.h>

SC_MODULE(m) {
    sc_signal<int> s;
    sc_event go;
    int runs;
    void worker() {
        while (true) {
            wait(go);
            runs++;
            wait(3, SC_NS);
            std::cout << sc_time_stamp() << " worker run " << runs << std::endl;
            s.write(runs);
        }
    }
    void watch() { std::cout << sc_time_stamp() << " s = " << s.read() << std::endl; }
    SC_CTOR(m) : runs(0) {
        SC_THREAD(worker);
        SC_METHOD(watch);
        sensitive << s;
        dont_initialize();
    }
};

int sc_main(int, char*[]) {
    m u("u");
    u.go.notify(2, SC_NS);
    sc_start();
    std::cout << "returned at " << sc_time_stamp() << std::endl;
    u.go.notify(5, SC_NS);
    sc_start();
    std::cout << "returned at " << sc_time_stamp() << std::endl;
    u.go.notify(1, SC_NS);
    sc_start(10, SC_NS);
    std::cout << "bounded run ends at " << sc_time_stamp() << ", runs " << u.runs << std::endl;
    return 0;
}
