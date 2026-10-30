// User structs as signal and FIFO payloads (IEEE 1666-2023 §6.4.2: the
// type needs operator==, operator<< and sc_trace): a signal notifies when
// operator== says the value changed; values are copied. The FIFO reader
// prints half a nanosecond later, so no two processes print in the same
// delta cycle (their order is unspecified, §4.2.1.2).
#include <systemc.h>
struct pkt {
    sc_uint<8> data;
    int id;
    bool last;
    pkt() : data(0), id(0), last(false) {}
    pkt(int d, int i) : data(d), id(i), last(false) {}
    bool operator==(const pkt& o) const { return data == o.data && id == o.id && last == o.last; }
    int sum() const { return data.to_int() + id; }
    void bump() { data = data + 1; }
};
inline std::ostream& operator<<(std::ostream& os, const pkt& p) {
    os << "{" << p.data << "," << p.id << (p.last ? ",last" : "") << "}";
    return os;
}
inline void sc_trace(sc_trace_file* tf, const pkt& p, const std::string& n) {
    sc_trace(tf, p.data, n + ".data");
    sc_trace(tf, p.id, n + ".id");
}
SC_MODULE(prod) {
    sc_out<pkt> out;
    sc_fifo_out<pkt> f;
    void run() {
        pkt p(1, 10);
        for (int i = 0; i < 3; i++) {
            out.write(p);
            f.write(p);
            p.bump();          // the copies written are not affected
            wait(1, SC_NS);
        }
        out.write(pkt(3, 10)); // equal to the last value: no event
        wait(1, SC_NS);
        pkt q = p;             // a copy
        q.id = 99;
        q.last = true;
        out.write(q);
        f.write(q);
        std::cout << sc_time_stamp() << " p=" << p << " q=" << q << " sum " << q.sum() << std::endl;
    }
    SC_CTOR(prod) { SC_THREAD(run); }
};
SC_MODULE(cons) {
    sc_in<pkt> in;
    sc_fifo_in<pkt> f;
    void on() { std::cout << sc_time_stamp() << " in=" << in.read() << std::endl; }
    void drain() {
        while (true) {
            pkt p = f.read();
            wait(500, SC_PS);
            std::cout << sc_time_stamp() << " fifo " << p << (p == in.read() ? " same" : " differs") << std::endl;
        }
    }
    SC_CTOR(cons) { SC_METHOD(on); sensitive << in; dont_initialize(); SC_THREAD(drain); }
};
int sc_main(int, char*[]) {
    sc_signal<pkt> s;
    sc_fifo<pkt> q(4);
    prod p("p");
    cons c("c");
    p.out(s); p.f(q);
    c.in(s); c.f(q);
    sc_start(10, SC_NS);
    std::cout << "final " << s.read() << std::endl;
    return 0;
}
