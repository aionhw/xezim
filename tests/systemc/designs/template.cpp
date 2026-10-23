// t10: template modules at two widths, constructor parameters, sc_vector
// ports bound to an sc_vector of signals
#include <systemc.h>
template <int W>
SC_MODULE(acc) {
    sc_in<bool> clk;
    sc_in<sc_uint<W> > d;
    sc_out<sc_uint<W> > q;
    void step() { q.write(q.read() + d.read()); }
    SC_CTOR(acc) { SC_METHOD(step); sensitive << clk.pos(); dont_initialize(); }
};
SC_MODULE(orr) {
    sc_vector<sc_in<bool> > in{"in", 3};
    sc_out<bool> o;
    void eval() { o.write(in[0].read() || in[1].read() || in[2].read()); }
    SC_CTOR(orr) { SC_METHOD(eval); for (int i = 0; i < 3; i++) sensitive << in[i]; }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<sc_uint<4> > d4, q4;
    sc_signal<sc_uint<8> > d8, q8;
    sc_vector<sc_signal<bool> > ins("ins", 3);
    sc_signal<bool> o;
    acc<4> a4("a4");
    acc<8> a8("a8");
    orr g("g");
    a4.clk(clk); a4.d(d4); a4.q(q4);
    a8.clk(clk); a8.d(d8); a8.q(q8);
    g.in(ins); g.o(o);
    d4 = 7; d8 = 7;
    sc_start(41, SC_NS);
    std::cout << "q4=" << q4.read() << " q8=" << q8.read() << " o=" << o.read() << std::endl;
    ins[1] = true;
    sc_start(1, SC_NS);
    std::cout << "o=" << o.read() << std::endl;
    return 0;
}
