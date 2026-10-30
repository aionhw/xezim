// C++ destruction: sc_main's modules are destroyed when it returns, last
// declared first; a module's member modules after its own destructor body,
// in reverse declaration order.
#include <systemc.h>

SC_MODULE(leaf) {
    int id;
    SC_CTOR(leaf) : id(0) {}
    ~leaf() { std::cout << "~leaf " << id << " " << name() << std::endl; }
};
SC_MODULE(node) {
    leaf a, b;
    int count;
    void run() { wait(3, SC_NS); count = 3; }
    SC_CTOR(node) : a("a"), b("b"), count(0) { a.id = 1; b.id = 2; SC_THREAD(run); }
    ~node() { std::cout << "~node " << name() << " count " << count << std::endl; }
};

int sc_main(int, char*[]) {
    node n1("n1");
    leaf l("l");
    node n2("n2");
    sc_start(10, SC_NS);
    std::cout << "sc_main returns" << std::endl;
    return 0;
}
