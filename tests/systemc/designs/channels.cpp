// §5.12–§5.15 interfaces, ports, exports and hierarchical channels: a FIFO
// channel implementing two interfaces; ports bound to it directly, through
// a parent's port and through exports (export to export); a multiport;
// static sensitivity to a port's event; a call through an export from
// sc_main. The FIFO's last batch has an even number of notifications. (How
// often the method runs within one time step depends on the order of
// processes in a delta cycle, so it reports when it last ran.)
#include <systemc.h>

class write_if : virtual public sc_interface {
public:
    virtual void write(char) = 0;
};
class read_if : virtual public sc_interface {
public:
    virtual void read(char&) = 0;
    virtual int num_available() = 0;
    virtual const sc_event& data_written_event() const = 0;
};

class fifo : public sc_channel, public write_if, public read_if {
public:
    fifo(sc_module_name n) : sc_channel(n), num(0), first(0) {}
    void write(char c) {
        if (num == max) wait(read_ev);
        data[(first + num) % max] = c;
        ++num;
        write_ev.notify();
    }
    void read(char& c) {
        if (num == 0) wait(write_ev);
        c = data[first];
        --num;
        first = (first + 1) % max;
        read_ev.notify();
    }
    int num_available() { return num; }
    const sc_event& data_written_event() const { return write_ev; }
    const sc_event& default_event() const { return write_ev; }
private:
    enum { max = 10 };
    char data[max];
    int num, first;
    sc_event write_ev, read_ev;
};

class counter_if : virtual public sc_interface {
public:
    virtual int bump(int by) = 0;
};
class counter : public sc_channel, public counter_if {
public:
    counter(sc_module_name n, int start) : sc_channel(n), value(start) {}
    int bump(int by) { value += by; return value; }
    int value;
};

SC_MODULE(producer) {
    sc_port<write_if> out;
    void run() {
        const char* s = "hello world\n";
        while (*s) out->write(*s++);
    }
    SC_CTOR(producer) { SC_THREAD(run); }
};

SC_MODULE(consumer) {
    sc_port<read_if> in;
    void run() {
        char c;
        while (true) {
            in->read(c);
            std::cout << c;
            if (in->num_available() == 1) std::cout << "<1>";
        }
    }
    SC_CTOR(consumer) { SC_THREAD(run); }
};

// A parent passing its port down: port to port.
SC_MODULE(wrapper) {
    sc_port<write_if> out;
    producer p;
    SC_CTOR(wrapper) : p("p") { p.out(out); }
};

// A module exporting a channel inside it, and one re-exporting that export.
SC_MODULE(store) {
    sc_export<read_if> rd;
    sc_export<write_if> wr;
    fifo f;
    SC_CTOR(store) : f("f") { rd(f); wr(f); }
};
SC_MODULE(outer) {
    sc_export<read_if> rd;
    sc_export<write_if> wr;
    store s;
    SC_CTOR(outer) : s("s") { rd(s.rd); wr.bind(s.wr); }
};

// A multiport on two counters, and a method sensitive to a port's event.
SC_MODULE(multi) {
    sc_port<counter_if, 2> cs;
    sc_port<read_if> watch;
    int changes;
    sc_time last;
    void run() {
        wait(1, SC_NS);
        for (int i = 0; i < cs.size(); i++)
            std::cout << "counter " << i << " -> " << cs[i]->bump(i + 1) << std::endl;
    }
    void seen() { changes++; last = sc_time_stamp(); }
    SC_CTOR(multi) : changes(0) {
        SC_THREAD(run);
        SC_METHOD(seen);
        sensitive << watch;     // the channel's default_event()
        dont_initialize();
    }
};

int sc_main(int, char*[]) {
    outer o("o");
    wrapper w("w");
    consumer c("c");
    counter c0("c0", 100), c1("c1", 200);
    multi m("m");
    w.out(o.wr);
    c.in(o.rd);
    m.cs(c0);
    m.cs(c1);
    m.watch(o.rd);
    sc_start(10, SC_NS);
    std::cout << "written event seen " << (m.changes > 0) << ", last at " << m.last << ", available "
              << o.rd->num_available() << std::endl;
    return 0;
}
