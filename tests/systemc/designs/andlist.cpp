// §5.10.7 waiting on event lists: `e1 & e2` resumes once both have been
// notified (in any order, at different times); `e1 | e2` on the first; with
// a timeout, on whichever comes first.
#include <systemc.h>

SC_MODULE(m) {
    sc_event a, b, c;
    void waiter() {
        wait(a & b);
        std::cout << sc_time_stamp() << " a & b" << std::endl;
        wait(a & b & c);
        std::cout << sc_time_stamp() << " a & b & c" << std::endl;
        wait(a | c);
        std::cout << sc_time_stamp() << " a | c" << std::endl;
        wait(100, SC_NS, a & b);
        std::cout << sc_time_stamp() << " timeout or a & b" << std::endl;
        wait(5, SC_NS, a & c);
        std::cout << sc_time_stamp() << " timeout or a & c" << std::endl;
    }
    void notifier() {
        wait(2, SC_NS); b.notify();
        wait(3, SC_NS); b.notify();
        wait(1, SC_NS); a.notify();      // 6 ns: a & b
        wait(1, SC_NS); c.notify();
        wait(1, SC_NS); b.notify();
        wait(1, SC_NS); a.notify();      // 9 ns: a & b & c
        wait(2, SC_NS); c.notify();      // 11 ns: a | c
        wait(1, SC_NS); a.notify();
        wait(4, SC_NS); b.notify();      // 16 ns: a & b
        wait(1, SC_NS); c.notify();      // a & c times out at 21 ns
    }
    SC_CTOR(m) { SC_THREAD(waiter); SC_THREAD(notifier); }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start(50, SC_NS);
    return 0;
}
