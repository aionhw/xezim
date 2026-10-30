// §4.4 elaboration and simulation callbacks: called at the first sc_start in
// hierarchy order (a module before its children), end_of_simulation after
// sc_stop; sc_main sees argc 1 (no arguments).
#include <systemc.h>
SC_MODULE(leaf) {
    sc_in<bool> in;
    void run() { std::cout << sc_time_stamp() << " " << name() << " runs" << std::endl; }
    void before_end_of_elaboration() { std::cout << name() << " before_end_of_elaboration" << std::endl; }
    void end_of_elaboration() { std::cout << name() << " end_of_elaboration" << std::endl; }
    void start_of_simulation() { std::cout << name() << " start_of_simulation at " << sc_time_stamp() << std::endl; }
    void end_of_simulation() { std::cout << name() << " end_of_simulation at " << sc_time_stamp() << std::endl; }
    SC_CTOR(leaf) { SC_METHOD(run); sensitive << in; }
};
SC_MODULE(top) {
    sc_signal<bool> s;
    leaf a, b;
    void end_of_elaboration() { std::cout << name() << " end_of_elaboration" << std::endl; }
    void end_of_simulation() { std::cout << name() << " end_of_simulation" << std::endl; }
    SC_CTOR(top) : a("a"), b("b") { a.in(s); b.in(s); }
};
int sc_main(int argc, char* argv[]) {
    top t("t");
    std::cout << "before sc_start" << std::endl;
    sc_start(5, SC_NS);
    std::cout << "between" << std::endl;
    sc_start(5, SC_NS);
    sc_stop();
    std::cout << "after, argc " << argc << std::endl;
    return 0;
}
