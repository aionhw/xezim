// t3: hierarchy — adder + register inside a top module, (a, b) concatenation
#include <systemc.h>

SC_MODULE(adder) {
    sc_in<sc_uint<8> > a, b;
    sc_out<sc_uint<9> > sum;
    void add() { sum.write(a.read() + b.read()); }
    SC_CTOR(adder) { SC_METHOD(add); sensitive << a << b; }
};

SC_MODULE(top) {
    sc_signal<sc_uint<8> > a, b;
    sc_signal<sc_uint<9> > sum;
    adder* add;
    void stim() {
        a = 200; b = 100;
        wait(1, SC_NS);
        sc_uint<9> s = sum.read();
        sc_uint<1> carry = static_cast<bool>(s[8]);
        sc_uint<8> low = s.range(7, 0);
        sc_uint<9> again = (carry, low);
        std::cout << "sum=" << s << " carry=" << carry << " low=" << low
                  << " concat=" << again << std::endl;
    }
    SC_CTOR(top) {
        add = new adder("add");
        add->a(a); add->b(b); add->sum(sum);
        SC_THREAD(stim);
    }
    ~top() { delete add; }
};

int sc_main(int, char*[]) {
    top t("t");
    sc_start(5, SC_NS);
    return 0;
}
