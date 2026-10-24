// Several translation units sharing a header: module declared in alu.h,
// its process defined in alu.cpp, testbench here.
#include "alu.h"
SC_MODULE(tb) {
    sc_out<sc_uint<8> > a, b;
    sc_out<int> op;
    sc_in<sc_uint<8> > y;
    void run() {
        int ops[3] = {OP_ADD, OP_SUB, OP_AND};
        for (int i = 0; i < 3; i++) {
            a.write(200);
            b.write(100 + i);
            op.write(ops[i]);
            wait(1, SC_NS);
            std::cout << "op " << i << " y=" << y.read() << std::endl;
        }
    }
    SC_CTOR(tb) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sc_signal<sc_uint<8> > a, b, y;
    sc_signal<int> op;
    alu u("u");
    tb t("t");
    u.a(a); u.b(b); u.op(op); u.y(y);
    t.a(a); t.b(b); t.op(op); t.y(y);
    sc_start(5, SC_NS);
    return 0;
}
