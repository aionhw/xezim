#ifndef ALU_H
#define ALU_H
#include <systemc.h>
enum op_t { OP_ADD, OP_SUB, OP_AND };
SC_MODULE(alu) {
    sc_in<sc_uint<8> > a, b;
    sc_in<int> op;
    sc_out<sc_uint<8> > y;
    void eval();
    SC_CTOR(alu) {
        SC_METHOD(eval);
        sensitive << a << b << op;
    }
};
#endif
