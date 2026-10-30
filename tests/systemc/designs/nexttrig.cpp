// §5.2.17 next_trigger(): a method's next activation waits for the time,
// event, event list or timeout it names, once; next_trigger() and a call
// without it return to the static sensitivity.
#include <systemc.h>

SC_MODULE(m) {
    sc_signal<int> s;
    sc_event a, b;
    int n;
    void meth() {
        std::cout << sc_time_stamp() << " meth " << n << " s=" << s.read() << std::endl;
        switch (n++) {
        case 0: next_trigger(3, SC_NS); break;            // a time
        case 1: next_trigger(a); break;                   // an event
        case 2: next_trigger(a & b); break;               // both events
        case 3: next_trigger(10, SC_NS, a | b); break;    // timeout or either
        case 4: next_trigger(SC_ZERO_TIME); break;        // the next delta
        case 5: break;                                    // static: s
        default: next_trigger(); break;                   // static again
        }
    }
    void stim() {
        wait(5, SC_NS); a.notify();       // 5: case 1 -> 2
        wait(2, SC_NS); b.notify();       // 7
        wait(2, SC_NS); a.notify();       // 9: a & b -> case 3
        wait(4, SC_NS); b.notify();       // 13: a | b -> case 4, then delta
        wait(2, SC_NS); s.write(1);       // 15: static
        wait(2, SC_NS); s.write(2);       // 17
        wait(2, SC_NS); a.notify();       // ignored
    }
    SC_CTOR(m) : n(0) {
        SC_METHOD(meth);
        sensitive << s;
        SC_THREAD(stim);
    }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start(30, SC_NS);
    return 0;
}
