// C++ templates (ISO/IEC 14882 §13): a module template with a type
// parameter instantiated at two types, class templates with type and
// value parameters (defaults, an instantiation as another's argument and
// as a base class, the injected class name).
#include <systemc.h>

template <class T, int N = 3>
class ring {
public:
    ring() : head(0), count(0) {}
    bool push(const T& v) {
        if (count == N) return false;
        buf[(head + count) % N] = v;
        count++;
        return true;
    }
    T pop() { T v = buf[head]; head = (head + 1) % N; count--; return v; }
    int size() const { return count; }
    ring copy_of() const { ring r = *this; return r; }
private:
    T buf[N];
    int head, count;
};

template <class T>
struct pair2 {
    T a, b;
    pair2() : a(), b() {}
    pair2(T x, T y) : a(x), b(y) {}
    T sum() const { return a + b; }
};

template <class T>
class counted : public ring<T, 2> {
public:
    counted() : pushes(0) {}
    void add(const T& v) { if (this->push(v)) pushes++; }
    int pushes;
};

template <class T, int DEPTH>
SC_MODULE(delay) {
    sc_in<bool> clk;
    sc_in<T> in;
    sc_out<T> out;
    sc_signal<T> stage[DEPTH];
    void step() {
        out.write(stage[DEPTH - 1].read());
        for (int i = DEPTH - 1; i > 0; i--) stage[i].write(stage[i - 1].read());
        stage[0].write(in.read());
    }
    SC_CTOR(delay) { SC_METHOD(step); sensitive << clk.pos(); }
};

SC_MODULE(tb) {
    sc_in<bool> clk;
    sc_out<int> a;
    sc_out<sc_uint<8> > b;
    sc_in<int> ra;
    sc_in<sc_uint<8> > rb;
    ring<pair2<int>, 4> pairs;
    void run() {
        ring<int> r;
        for (int i = 1; i <= 4; i++)
            std::cout << "push " << i << (r.push(i * 10) ? " ok" : " full") << std::endl;
        ring<int> c = r.copy_of();
        std::cout << "pop " << r.pop() << " size " << r.size() << " copy size " << c.size() << std::endl;
        pairs.push(pair2<int>(2, 3));
        pairs.push(pair2<int>(5, 7));
        std::cout << "pair sum " << pairs.pop().sum() << " then " << pairs.pop().sum() << std::endl;
        counted<double> k;
        k.add(1.5); k.add(2.5); k.add(3.5);
        std::cout << "counted " << k.pushes << " size " << k.size() << " first " << k.pop() << std::endl;
        for (int i = 0; i < 6; i++) {
            a.write(i);
            b.write(100 + i);
            wait();
            std::cout << sc_time_stamp() << " ra=" << ra.read() << " rb=" << rb.read() << std::endl;
        }
    }
    SC_CTOR(tb) { SC_THREAD(run); sensitive << clk.pos(); }
};

int sc_main(int, char*[]) {
    sc_clock clk("clk", 10, SC_NS);
    sc_signal<int> a, ra;
    sc_signal<sc_uint<8> > b, rb;
    delay<int, 2> d1("d1");
    delay<sc_uint<8>, 3> d2("d2");
    tb t("t");
    d1.clk(clk); d1.in(a); d1.out(ra);
    d2.clk(clk); d2.in(b); d2.out(rb);
    t.clk(clk); t.a(a); t.b(b); t.ra(ra); t.rb(rb);
    sc_start(70, SC_NS);
    return 0;
}
