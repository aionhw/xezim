// t4: producer/consumer over sc_fifo (SC_THREAD, sc_fifo_out/in ports)
#include <systemc.h>

SC_MODULE(producer) {
    sc_fifo_out<int> out;
    void run() {
        for (int i = 1; i <= 5; i++) { out.write(i * i); wait(2, SC_NS); }
    }
    SC_CTOR(producer) { SC_THREAD(run); }
};

SC_MODULE(consumer) {
    sc_fifo_in<int> in;
    void run() {
        int total = 0;
        for (int i = 0; i < 5; i++) total += in.read();
        std::cout << "sum of squares 1..5 = " << total << " at " << sc_time_stamp() << std::endl;
    }
    SC_CTOR(consumer) { SC_THREAD(run); }
};

int sc_main(int, char*[]) {
    sc_fifo<int> f("f", 2);
    producer p("p");
    consumer c("c");
    p.out(f);
    c.in(f);
    sc_start();
    return 0;
}
