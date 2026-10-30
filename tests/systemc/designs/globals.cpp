// Namespace-scope variables, arrays (sized by their initializer),
// constants and references; free functions overloaded by their number of
// parameters (with a default argument).
#include <systemc.h>

static const char* names[] = { "INFO", "WARNING", "ERROR" };
const char* tag = "T";
int counts[3] = { 4, 5, 6 };
int& middle = counts[1];
static int calls = 0;
const double scale = 2.5;

int add(int a) { calls++; return a + 1; }
int add(int a, int b, int c = 100) { calls++; return a + b + c; }
void show() { std::cout << tag << " calls " << calls << std::endl; }
void show(const char* what) { std::cout << tag << " " << what << " middle " << middle << std::endl; }

SC_MODULE(m) {
    void run() {
        wait(1, SC_NS);
        counts[2] += add(1);
        show("in process");
        std::cout << names[1] << " " << counts[2] << " " << scale * 2 << std::endl;
    }
    SC_CTOR(m) { SC_THREAD(run); }
};

int sc_main(int, char*[]) {
    m u("u");
    std::cout << names[0] << " " << add(1, 2) << " " << add(1, 2, 3) << std::endl;
    counts[1] = 50;
    show("before");
    sc_start();
    show();
    return 0;
}
