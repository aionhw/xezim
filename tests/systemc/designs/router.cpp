// TLM-2.0 LT interconnect: an initiator talks to a router whose target
// socket forwards each transaction to one of two memories by address,
// translating the address; delays accumulate along the path.
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>
#include <cstring>
struct mem : sc_module {
    tlm_utils::simple_target_socket<mem> sock;
    unsigned char data[256];
    int id;
    SC_HAS_PROCESS(mem);
    mem(sc_module_name n, int i) : sc_module(n), sock("sock"), id(i) {
        sock.register_b_transport(this, &mem::b_transport);
        memset(data, 0, 256);
    }
    void b_transport(tlm::tlm_generic_payload& gp, sc_time& delay) {
        sc_dt::uint64 a = gp.get_address();
        if (gp.is_write()) memcpy(&data[a], gp.get_data_ptr(), gp.get_data_length());
        else memcpy(gp.get_data_ptr(), &data[a], gp.get_data_length());
        std::cout << sc_time_stamp() << " mem" << id << (gp.is_write() ? " write @" : " read @") << a << std::endl;
        delay += sc_time(20 + 10 * id, SC_NS);
        gp.set_response_status(tlm::TLM_OK_RESPONSE);
    }
};
struct router : sc_module {
    tlm_utils::simple_target_socket<router> in;
    tlm_utils::simple_initiator_socket<router> out0, out1;
    SC_CTOR(router) : in("in"), out0("out0"), out1("out1") {
        in.register_b_transport(this, &router::b_transport);
    }
    void b_transport(tlm::tlm_generic_payload& gp, sc_time& delay) {
        sc_dt::uint64 a = gp.get_address();
        delay += sc_time(5, SC_NS);
        gp.set_address(a & 0xFF);
        if (a < 0x100) out0->b_transport(gp, delay);
        else out1->b_transport(gp, delay);
        gp.set_address(a);
    }
};
struct cpu : sc_module {
    tlm_utils::simple_initiator_socket<cpu> sock;
    SC_CTOR(cpu) : sock("sock") { SC_THREAD(run); }
    void access(bool wr, sc_dt::uint64 a, unsigned int& v) {
        tlm::tlm_generic_payload gp;
        sc_time d = SC_ZERO_TIME;
        gp.set_command(wr ? tlm::TLM_WRITE_COMMAND : tlm::TLM_READ_COMMAND);
        gp.set_address(a);
        gp.set_data_ptr(reinterpret_cast<unsigned char*>(&v));
        gp.set_data_length(4);
        gp.set_streaming_width(4);
        gp.set_response_status(tlm::TLM_INCOMPLETE_RESPONSE);
        sock->b_transport(gp, d);
        wait(d);
    }
    void run() {
        unsigned int v;
        for (int i = 0; i < 4; i++) {
            v = 0x1000 * (i + 1) + i;
            access(true, (i % 2) * 0x100 + 4 * i, v);
        }
        for (int i = 0; i < 4; i++) {
            v = 0;
            access(false, (i % 2) * 0x100 + 4 * i, v);
            std::cout << sc_time_stamp() << " cpu read " << std::hex << v << std::dec << std::endl;
        }
    }
};
int sc_main(int, char*[]) {
    cpu c("c");
    router r("r");
    mem m0("m0", 0), m1("m1", 1);
    c.sock.bind(r.in);
    r.out0.bind(m0.sock);
    r.out1.bind(m1.sock);
    sc_start();
    std::cout << "end " << sc_time_stamp() << std::endl;
    return 0;
}
