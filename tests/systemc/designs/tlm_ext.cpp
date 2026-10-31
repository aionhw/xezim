// TLM-2.0 generic payload extensions (§14.21), memory management (§14.5:
// a tlm_mm_interface, acquire/release reference counts, free when the count
// falls to zero), and analysis ports (§17: write to every subscriber).
#include <systemc.h>
#include <tlm.h>

using namespace tlm;

struct tag_ext : tlm_extension<tag_ext> {
    int id;
    tag_ext() : id(0) {}
    tlm_extension_base* clone() const { tag_ext* e = new tag_ext; e->id = id; return e; }
    void copy_from(const tlm_extension_base& o) { id = static_cast<const tag_ext&>(o).id; }
};

struct pool : tlm_mm_interface {
    int frees;
    pool() : frees(0) {}
    void free(tlm_generic_payload* t) {
        frees++;
        std::cout << "freed addr " << t->get_address() << ", frees " << frees << std::endl;
        t->reset();
    }
};

struct logger : tlm_analysis_if<int> {
    const char* who;
    int sum;
    logger(const char* w) : who(w), sum(0) {}
    void write(const int& v) { sum += v; std::cout << who << " saw " << v << std::endl; }
};

SC_MODULE(m) {
    tlm_analysis_port<int> ap;
    pool mm;
    logger a, b;
    void run() {
        tlm_generic_payload* t = new tlm_generic_payload(&mm);
        t->set_address(64);
        t->acquire();
        std::cout << "has mm " << t->has_mm() << " refs " << t->get_ref_count() << std::endl;
        tag_ext* e = new tag_ext;
        e->id = 7;
        t->set_extension(e);
        tag_ext* g = 0;
        t->get_extension(g);
        std::cout << "extension id " << (g ? g->id : -1) << std::endl;
        tlm_generic_payload copy;
        copy.deep_copy_from(*t);
        tag_ext* h = copy.get_extension<tag_ext>();
        std::cout << "copied extension id " << (h ? h->id : -1) << (h == g ? " same" : " distinct") << std::endl;
        t->clear_extension(e);
        t->get_extension(g);
        std::cout << "after clear " << (g ? "present" : "absent") << std::endl;
        t->acquire();
        t->release();
        std::cout << "refs " << t->get_ref_count() << std::endl;
        t->release();
        ap.write(3);
        ap.write(4);
        wait(1, SC_NS);
        std::cout << "sums " << a.sum << " " << b.sum << std::endl;
    }
    SC_CTOR(m) : ap("ap"), a("a"), b("b") {
        ap.bind(a);
        ap.bind(b);
        SC_THREAD(run);
    }
};

int sc_main(int, char*[]) {
    m u("u");
    sc_start();
    return 0;
}
