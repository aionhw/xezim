// §6.13–§6.18 resolved signals and ports: every process writing a resolved
// signal is a driver (directly or through a port); the value resolves bit
// by bit (`0`/`1` conflict → X, Z yields) and reads X before any write.
// One monitor prints, so no two processes print in the same delta cycle
// (their order is unspecified, §4.2.1.2).
#include <systemc.h>
SC_MODULE(drv) {
    sc_out_resolved o;
    sc_inout_rv<4> io;
    sc_logic v1, v2;
    sc_lv<4> b1;
    void run() {
        wait(1, SC_NS);
        o.write(v1);
        io.write(b1);
        wait(2, SC_NS);
        o.write(v2);
        io.write("ZZZZ");
    }
    SC_CTOR(drv) { SC_THREAD(run); }
};
SC_MODULE(probe) {
    sc_in_resolved i;
    sc_out<sc_logic> seen;
    void on() { seen.write(i.read()); }
    SC_CTOR(probe) { SC_METHOD(on); sensitive << i; dont_initialize(); }
};
SC_MODULE(top) {
    sc_signal_resolved s;
    sc_signal_rv<4> bus;
    sc_signal<sc_logic> seen;
    drv a, b;
    probe p;
    int posedges;
    void mon() {
        std::cout << sc_time_stamp() << " s=" << s.read() << " bus=" << bus.read()
                  << " seen=" << seen.read() << std::endl;
    }
    void pe() { posedges++; }
    void own() {
        std::cout << "init s=" << s.read() << " bus=" << bus.read() << std::endl;
        wait(4, SC_NS);
        s.write(SC_LOGIC_1);
        bus.write("1ZZ0");
        wait(1, SC_NS);
        s.write(SC_LOGIC_Z);
    }
    SC_CTOR(top) : a("a"), b("b"), p("p"), posedges(0) {
        a.o(s); b.o(s); a.io(bus); b.io(bus); p.i(s); p.seen(seen);
        a.v1 = SC_LOGIC_0; a.v2 = SC_LOGIC_Z; a.b1 = "01ZZ";
        b.v1 = SC_LOGIC_Z; b.v2 = SC_LOGIC_Z; b.b1 = "ZZ10";
        SC_METHOD(mon); sensitive << s << bus << seen; dont_initialize();
        SC_METHOD(pe); sensitive << s.posedge_event(); dont_initialize();
        SC_THREAD(own);
    }
};
int sc_main(int, char*[]) {
    top t("t");
    sc_start(10, SC_NS);
    std::cout << "end s=" << t.s.read() << " bus=" << t.bus.read()
              << " posedges=" << t.posedges << std::endl;
    return 0;
}
