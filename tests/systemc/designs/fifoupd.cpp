// §6.23 sc_fifo: a value written becomes readable, and the space freed by a
// read writable, only after the update phase.
#include <systemc.h>

SC_MODULE(m) {
    sc_fifo<int> f;
    void run() {
        f.write(1);
        std::cout << "after write: available " << f.num_available() << " free " << f.num_free() << std::endl;
        wait(SC_ZERO_TIME);
        std::cout << "next delta: available " << f.num_available() << " free " << f.num_free() << std::endl;
        int v;
        std::cout << "nb_read " << f.nb_read(v) << " v=" << v << std::endl;
        std::cout << "after read: available " << f.num_available() << " free " << f.num_free() << std::endl;
        f.write(2); f.write(3);
        std::cout << "two writes: free " << f.num_free() << " nb_write " << f.nb_write(4) << std::endl;
        wait(SC_ZERO_TIME);
        std::cout << "next delta: available " << f.num_available() << " free " << f.num_free() << std::endl;
    }
    SC_CTOR(m) : f(3) { SC_THREAD(run); }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start(1, SC_NS);
    return 0;
}
