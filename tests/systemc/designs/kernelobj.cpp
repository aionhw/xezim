// §6.27–§6.29 sc_mutex, sc_semaphore and sc_event_queue. The threads take
// turns at distinct times, so no two print in the same delta cycle.
#include <systemc.h>

SC_MODULE(m) {
    sc_mutex mx;
    sc_semaphore sem;
    sc_event_queue eq;
    int fires;
    void user(int id, int start) {
        wait(start, SC_NS);
        mx.lock();
        std::cout << sc_time_stamp() << " t" << id << " locked" << std::endl;
        wait(10, SC_NS);
        std::cout << sc_time_stamp() << " t" << id << " unlock " << mx.unlock() << std::endl;
    }
    void t1() { user(1, 1); }
    void t2() { user(2, 2); }
    void t3() {
        wait(3, SC_NS);
        std::cout << sc_time_stamp() << " trylock " << mx.trylock() << " unlock by non-owner " << mx.unlock() << std::endl;
    }
    void sems() {
        std::cout << "sem " << sem.get_value() << " trywait " << sem.trywait() << " " << sem.trywait()
                  << " -> " << sem.get_value() << std::endl;
        sem.post();
        sem.wait();
        std::cout << "sem after post+wait " << sem.get_value() << std::endl;
        eq.notify(5, SC_NS);
        eq.notify(7, SC_NS);
        eq.notify(9, SC_NS);
        wait(6, SC_NS);
        eq.cancel_all();
    }
    void on_eq() { fires++; std::cout << sc_time_stamp() << " event queue fired" << std::endl; }
    SC_CTOR(m) : sem(2), fires(0) {
        SC_THREAD(t1); SC_THREAD(t2); SC_THREAD(t3); SC_THREAD(sems);
        SC_METHOD(on_eq); sensitive << eq; dont_initialize();
    }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start(50, SC_NS);
    std::cout << "event queue fires " << u.fires << std::endl;
    return 0;
}
