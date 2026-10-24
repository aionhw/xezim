// C++ in process bodies: free functions, reference parameters, static
// locals, <cmath>, nested loops with break/continue, ternaries.
#include <systemc.h>
#include <cmath>
int gcd(int a, int b) {
    while (b != 0) {
        int t = a % b;
        a = b;
        b = t;
    }
    return a;
}
const int N = 6;
SC_MODULE(f) {
    sc_in<bool> clk;
    void swap(int& a, int& b) {
        int t = a;
        a = b;
        b = t;
    }
    int counter() {
        static int calls = 0;
        calls++;
        return calls;
    }
    void run() {
        int v[N] = {5, 3, 9, 1, 7, 2};
        for (int i = 0; i < N; i++)
            for (int j = 0; j + 1 < N - i; j++)
                if (v[j] > v[j + 1]) swap(v[j], v[j + 1]);
        for (int i = 0; i < N; i++) std::cout << v[i] << (i + 1 < N ? "," : "\n");
        std::cout << "gcd=" << gcd(84, 36) << " sqrt=" << std::sqrt(2.0) << " pow=" << pow(2.0, 10) << std::endl;
        int odd = 0;
        for (int i = 0; i < 20; i++) {
            if (i % 2 == 0) continue;
            if (i > 13) break;
            odd += i;
        }
        std::cout << "odd=" << odd << " abs=" << abs(-4) << " floor=" << floor(2.7) << std::endl;
        while (true) {
            wait();
            int c = counter();
            std::cout << sc_time_stamp() << " call " << c << std::endl;
            if (c == 3) break;
        }
    }
    SC_CTOR(f) {
        SC_THREAD(run);
        sensitive << clk.pos();
    }
};
int sc_main(int, char*[]) {
    sc_clock clk("clk", 4, SC_NS);
    f x("x");
    x.clk(clk);
    sc_start(20, SC_NS);
    return 0;
}
