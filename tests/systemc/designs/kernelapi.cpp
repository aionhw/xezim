// §5.6 process handles of static processes (sc_get_current_process_handle in
// a process and in a constructor), suspend/resume/kill, name, proc_kind;
// §4.5.8 sc_is_running/sc_get_status; §5.16 the object hierarchy
// (sc_get_top_level_objects, get_child_objects, sc_find_object).
#define SC_INCLUDE_DYNAMIC_PROCESSES
#include <systemc.h>
SC_MODULE(leaf) {
    sc_in<bool> in;
    sc_signal<int> s;
    void m() {}
    SC_CTOR(leaf) { SC_METHOD(m); sensitive << in; }
};
SC_MODULE(top) {
    sc_signal<bool> sig;
    sc_out<int> o;
    leaf a;
    sc_fifo<int> f;
    sc_clock clk;
    sc_process_handle th;
    void ticker() { while (true) { wait(2, SC_NS); std::cout << sc_time_stamp() << " tick, running " << sc_is_running() << " status " << sc_get_status() << std::endl; } }
    void boss() {
        sc_process_handle me = sc_get_current_process_handle();
        std::cout << "boss " << me.name() << " kind " << me.proc_kind() << std::endl;
        wait(3, SC_NS); th.suspend(); std::cout << sc_time_stamp() << " suspended" << std::endl;
        wait(4, SC_NS); th.resume(); std::cout << sc_time_stamp() << " resumed" << std::endl;
        wait(3, SC_NS); th.kill(); std::cout << sc_time_stamp() << " killed, terminated " << th.terminated() << std::endl;
    }
    SC_CTOR(top) : a("a"), f(2), clk("clk", 10, SC_NS) {
        a.in(sig);
        SC_THREAD(ticker);
        th = sc_get_current_process_handle();
        SC_THREAD(boss);
    }
};
void dump(const std::vector<sc_object*>& v, int d) {
    for (size_t i = 0; i < v.size(); i++) {
        for (int k = 0; k < d; k++) std::cout << "  ";
        std::cout << v[i]->name() << " (" << v[i]->kind() << ") base " << v[i]->basename() << std::endl;
        dump(v[i]->get_child_objects(), d + 1);
    }
}
int sc_main(int, char*[]) {
    sc_signal<int> outsig("outsig");
    top t("t");
    t.o(outsig);
    std::cout << "running " << sc_is_running() << " status " << sc_get_status() << std::endl;
    dump(sc_get_top_level_objects(), 0);
    sc_object* o = sc_find_object("t.a.s");
    std::cout << "found " << (o ? o->name() : "none") << " " << (sc_find_object("t.nope") == 0) << std::endl;
    sc_start(15, SC_NS);
    std::cout << "paused: running " << sc_is_running() << " status " << sc_get_status() << std::endl;
    sc_stop();
    std::cout << "stopped: running " << sc_is_running() << " status " << sc_get_status() << std::endl;
    return 0;
}
