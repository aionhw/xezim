// §5.5–§5.6 dynamic threads: sc_spawn of member and free functions through
// sc_bind (arguments copied at the spawn, sc_ref/sc_cref by reference), a
// returned value through `&r`, SC_FORK/SC_JOIN, process handles and
// terminated_event(), local events passed by reference.
#define SC_INCLUDE_DYNAMIC_PROCESSES
#include <systemc.h>

int twice(int x) { wait(1, SC_NS); return 2 * x; }
void show(const double& d) { std::cout << sc_time_stamp() << " show " << d << std::endl; }

SC_MODULE(m) {
    int total;
    int pass(const char* who, sc_event& rcv, sc_event& snd, int cnt) {
        while (--cnt >= 0) {
            wait(rcv);
            std::cout << sc_time_stamp() << " " << who << std::endl;
            wait(10, SC_NS);
            snd.notify();
        }
        return cnt;
    }
    int later(int i) {
        wait(i + 1, SC_NS);
        std::cout << sc_time_stamp() << " later " << i << std::endl;
        total += i;
        return i;
    }
    void main() {
        int r = -5;
        sc_event e1, e2;
        e1.notify(100, SC_NS);
        SC_FORK
            sc_spawn(&r, sc_bind(&m::pass, this, "one", sc_ref(e1), sc_ref(e2), 2), "p1"),
            sc_spawn(&r, sc_bind(&m::pass, this, "two", sc_ref(e2), sc_ref(e1), 2), "p2"),
        SC_JOIN
        std::cout << sc_time_stamp() << " joined, r " << r << std::endl;
        for (int i = 0; i < 3; i++)
            sc_spawn(sc_bind(&m::later, this, i));
        wait(10, SC_NS);
        std::cout << sc_time_stamp() << " total " << total << std::endl;
        sc_process_handle h = sc_spawn(&r, sc_bind(&twice, 21));
        std::cout << "terminated " << h.terminated() << std::endl;
        wait(h.terminated_event());
        std::cout << sc_time_stamp() << " twice " << r << " terminated " << h.terminated() << std::endl;
        double d = 1.5;
        sc_spawn_options o;
        o.set_stack_size(0);
        wait(sc_spawn(sc_bind(&show, sc_cref(d)), "show", &o).terminated_event());
        std::cout << "int(9.87) " << int(9.87) << std::endl;
    }
    SC_CTOR(m) : total(0) { SC_THREAD(main); }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start();
    return 0;
}
