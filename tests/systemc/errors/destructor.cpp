#include <systemc.h>
struct tracker {
    int id;
    tracker(int i) : id(i) {}
    ~tracker() { std::cout << "gone " << id << std::endl; }
};
SC_MODULE(m) {
    void run() { tracker t(1); wait(1, SC_NS); }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) { m u("u"); sc_start(); return 0; }
