// §4.2.1.3 a zero-time wait and a delta notification take effect in the
// next delta cycle, after the update phase: a signal written before them
// reads its new value.
#include <systemc.h>
SC_MODULE(m) {
    sc_signal<int> s;
    sc_event e;
    void run() {
        s.write(5);
        wait(SC_ZERO_TIME);
        std::cout << "after zero wait " << s.read() << std::endl;
        e.notify(SC_ZERO_TIME);
        s.write(6);
        wait(e);
        std::cout << "after delta event " << s.read() << std::endl;
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) { m u("u"); sc_start(1, SC_NS); return 0; }
