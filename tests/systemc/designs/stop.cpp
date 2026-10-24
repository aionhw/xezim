// sc_start() without a limit, ended by sc_stop() from a thread; statements
// after sc_start; timed and zero-time notifications.
#include <systemc.h>
SC_MODULE(ctl) {
    sc_event go, tick;
    int n;
    void producer() {
        while (true) {
            wait(3, SC_NS);
            n++;
            tick.notify(SC_ZERO_TIME);
            if (n == 4) {
                go.notify(2, SC_NS);
            }
        }
    }
    void consumer() {
        while (true) {
            wait(tick);
            std::cout << sc_time_stamp() << " tick " << n << std::endl;
        }
    }
    void finisher() {
        wait(go);
        std::cout << sc_time_stamp() << " stopping" << std::endl;
        sc_stop();
    }
    SC_CTOR(ctl) : n(0) {
        SC_THREAD(producer);
        SC_THREAD(consumer);
        SC_THREAD(finisher);
    }
};
int sc_main(int, char*[]) {
    ctl c("c");
    std::cout << "start" << std::endl;
    sc_start();
    std::cout << "after sc_start at " << sc_time_stamp() << " n=" << c.n << std::endl;
    return 0;
}
