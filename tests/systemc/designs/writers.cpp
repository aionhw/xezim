// §6.4.4 writer policy: SC_MANY_WRITERS lets several processes write one
// signal; §5.16 object names: an object constructed without one gets a
// generated name per kind (signal_0, port_0, …) within its parent.
#include <systemc.h>
SC_MODULE(m) {
    sc_in<bool> clk;
    sc_out<int> q;
    sc_signal<int> a;
    sc_signal<int> b{"bee"};
    sc_signal<int, SC_MANY_WRITERS> shared;
    sc_fifo<int> f;
    void p1() { wait(1, SC_NS); shared.write(1); }
    void p2() { wait(2, SC_NS); shared.write(2); }
    void on() { std::cout << sc_time_stamp() << " shared=" << shared.read() << std::endl; }
    SC_CTOR(m) : q("q") {
        SC_THREAD(p1); SC_THREAD(p2);
        SC_METHOD(on); sensitive << shared; dont_initialize();
    }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<int> q;
    sc_signal<int> anon;
    m u("u");
    u.clk(clk); u.q(q);
    std::cout << u.name() << " " << u.clk.name() << " " << u.q.name() << " " << u.a.name()
              << " " << u.b.name() << " " << u.shared.name() << " " << u.f.name() << std::endl;
    std::cout << clk.name() << " " << q.name() << " " << anon.name() << " " << u.a.basename() << std::endl;
    sc_start(5, SC_NS);
    return 0;
}
