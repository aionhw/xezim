// sc_fifo of depth 2 with a fast producer and slow consumer: blocking
// write/read, num_available/num_free, nb_read on an empty fifo.
#include <systemc.h>
SC_MODULE(prod) {
    sc_fifo_out<int> out;
    void run() {
        for (int i = 1; i <= 5; i++) {
            out.write(i * 10);
            std::cout << sc_time_stamp() << " wrote " << i * 10 << std::endl;
        }
    }
    SC_CTOR(prod) { SC_THREAD(run); }
};
SC_MODULE(cons) {
    sc_fifo_in<int> in;
    void run() {
        int v;
        wait(1, SC_NS);
        std::cout << "avail " << in.num_available() << std::endl;
        for (int i = 0; i < 5; i++) {
            v = in.read();
            std::cout << sc_time_stamp() << " read " << v << std::endl;
            wait(4, SC_NS);
        }
        bool got = in.nb_read(v);
        std::cout << sc_time_stamp() << " nb_read " << got << std::endl;
    }
    SC_CTOR(cons) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_fifo<int> f(2);
    prod p("p");
    cons c("c");
    p.out(f);
    c.in(f);
    sc_start();
    std::cout << "done " << sc_time_stamp() << std::endl;
    return 0;
}
