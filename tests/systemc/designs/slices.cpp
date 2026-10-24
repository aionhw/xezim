// Bit-level operations on sc_uint/sc_int/sc_biguint: shifts, masks, signed
// arithmetic, ranges, concatenation as lvalue source, reductions, wrap.
#include <systemc.h>
SC_MODULE(sl) {
    void run() {
        sc_uint<16> a = 0xBEEF;
        sc_uint<4> n0 = a.range(3, 0), n3 = a.range(15, 12);
        sc_uint<8> sw = (n0, n3);
        std::cout << std::hex << "a=" << a << " n0=" << n0 << " n3=" << n3 << " sw=" << sw << std::dec << std::endl;
        sc_int<8> s = -100;
        sc_int<8> t = s - 50;
        std::cout << "s=" << s << " t=" << t << " s>>2=" << (s >> 2) << std::endl;
        sc_uint<8> m = 0;
        m[0] = 1;
        m[7] = 1;
        m.range(4, 2) = 5;
        std::cout << "m=" << m << " or=" << m.or_reduce() << " and=" << m.and_reduce() << std::endl;
        sc_biguint<80> big = 1;
        big = big << 70;
        big = big + 3;
        std::cout << "big_hi=" << big.range(79, 64).to_uint() << " big_lo=" << big.range(7, 0).to_uint() << std::endl;
        sc_uint<4> w = 15;
        w++;
        std::cout << "w=" << w << " ~a=" << (sc_uint<16>)(~a) << " a^0xFF=" << (a ^ 0xFF) << std::endl;
    }
    SC_CTOR(sl) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    sl x("x");
    sc_start(1, SC_NS);
    return 0;
}
