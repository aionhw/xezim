// t8: data types and C++ statements: sc_bv/sc_lv/sc_int printing, hex,
// sc_time arithmetic, switch fall-through, loops, value-returning members
#include <systemc.h>
#include <cstdio>
SC_MODULE(ty) {
    int classify(int v) {
        int r = 0;
        switch (v) {
        case 0: r += 1;
        case 1: r += 10; break;
        case 2:
        case 3: r = 100; break;
        default: r = -1;
        }
        return r;
    }
    sc_uint<8> rev(sc_uint<8> x) {
        sc_uint<8> y = 0;
        for (int i = 0; i < 8; i++) y[7 - i] = x[i];
        return y;
    }
    void run() {
        sc_bv<8> bv = 0xA5;
        sc_lv<4> lv = 0x9;
        sc_int<6> s = -5;
        sc_uint<12> u = 0xABC;
        std::cout << "bv=" << bv << " lv=" << lv << " s=" << s << " u=" << u << std::endl;
        std::cout << std::hex << "hex u=" << u << " 255=" << 255 << std::dec << " dec=" << 255 << std::endl;
        std::cout << "range=" << u.range(7, 4) << " bit3=" << u[3] << " xor=" << u.xor_reduce() << std::endl;
        sc_time t = sc_time(1500, SC_PS);
        t += sc_time(2, SC_NS);
        std::cout << "t=" << t << " double=" << t.to_seconds() * 1e9 << std::endl;
        for (int v = 0; v < 5; v++) std::cout << classify(v) << " ";
        std::cout << std::endl;
        int n = 0;
        do { n += 3; } while (n < 10);
        while (n > 4) n -= 4;
        std::cout << "n=" << n << " rev=" << rev(0x0F) << std::endl;
        printf("printf %d %x %s %c\n", 42, 255, "str", 'Z');
        double d = 2.5;
        std::cout << "d*3=" << d * 3 << " int=" << (int)(d * 3) << std::endl;
        bool b = (u > 100) && !(s > 0);
        std::cout << "b=" << b << std::endl;
    }
    SC_CTOR(ty) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    ty t("t");
    sc_start(1, SC_NS);
    return 0;
}
