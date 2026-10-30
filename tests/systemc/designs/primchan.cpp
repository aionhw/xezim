// §5.15 a primitive channel: write() stores the new value and requests the
// update phase; update() makes it current and notifies (once per delta,
// however many writes).
#include <systemc.h>

class reg_if : virtual public sc_interface {
public:
    virtual void write(int v) = 0;
    virtual int read() const = 0;
    virtual const sc_event& default_event() const = 0;
};

class reg : public sc_prim_channel, public reg_if {
public:
    reg(const char* n) : sc_prim_channel(n), cur(0), next(0), updates(0) {}
    void write(int v) { next = v; request_update(); }
    int read() const { return cur; }
    const sc_event& default_event() const { return changed; }
    int updates;
protected:
    void update() {
        updates++;
        if (next != cur) { cur = next; changed.notify(SC_ZERO_TIME); }
    }
private:
    int cur, next;
    sc_event changed;
};

SC_MODULE(writer) {
    sc_port<reg_if> r;
    void run() {
        for (int i = 1; i <= 3; i++) {
            r->write(i * 10);
            r->write(i * 10 + 1);     // the last write of the delta counts
            std::cout << sc_time_stamp() << " wrote, still reads " << r->read() << std::endl;
            wait(5, SC_NS);
        }
    }
    SC_CTOR(writer) { SC_THREAD(run); }
};

SC_MODULE(reader) {
    sc_port<reg_if> r;
    void on() { std::cout << sc_time_stamp() << " changed to " << r->read() << std::endl; }
    SC_CTOR(reader) { SC_METHOD(on); sensitive << r; dont_initialize(); }
};

int sc_main(int, char*[]) {
    reg q("q");
    writer w("w");
    reader rd("rd");
    w.r(q);
    rd.r(q);
    sc_start(20, SC_NS);
    std::cout << "updates " << q.updates << std::endl;
    return 0;
}
