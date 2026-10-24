#include "alu.h"
void alu::eval() {
    switch (op.read()) {
    case OP_ADD: y.write(a.read() + b.read()); break;
    case OP_SUB: y.write(a.read() - b.read()); break;
    default: y.write(a.read() & b.read()); break;
    }
}
