// t6: sc_event notify forms, event OR-lists, timeouts, SC_THREAD chains
#include <systemc.h>
SC_MODULE(ev) {
    sc_event go, ack, never;
    int count = 0;
    void producer() {
        for (int i = 0; i < 3; i++) {
            wait(10, SC_NS);
            std::cout << sc_time_stamp() << " notify go " << i << std::endl;
            go.notify();
            wait(ack);
            std::cout << sc_time_stamp() << " got ack " << i << std::endl;
        }
        go.notify(5, SC_NS);
        wait(20, SC_NS);
        ack.notify(SC_ZERO_TIME);
    }
    void consumer() {
        while (true) {
            wait(go | never);
            count++;
            std::cout << sc_time_stamp() << " consumer woke count=" << count << std::endl;
            ack.notify(2, SC_NS);
        }
    }
    void timeout() {
        wait(sc_time(100, SC_NS), never);
        std::cout << sc_time_stamp() << " timeout fired" << std::endl;
    }
    SC_CTOR(ev) { SC_THREAD(producer); SC_THREAD(consumer); SC_THREAD(timeout); }
};
int sc_main(int, char*[]) {
    ev e("e");
    sc_start(200, SC_NS);
    std::cout << "done at " << sc_time_stamp() << std::endl;
    return 0;
}
