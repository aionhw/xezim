// TLM-2.0 LT: transport_dbg, tlm_initiator_socket/tlm_target_socket via
// simple sockets, byte and word pointer views of target memory, memset,
// address errors and the timing annotation.
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>
#include <cstring>
struct ram : sc_module {
    tlm_utils::simple_target_socket<ram> sock;
    unsigned char mem[64];
    SC_CTOR(ram) : sock("sock") {
        sock.register_b_transport(this, &ram::b_transport);
        sock.register_transport_dbg(this, &ram::transport_dbg);
        memset(mem, 0, 64);
        for (int i = 0; i < 16; i++) mem[i] = i * 3;
    }
    void b_transport(tlm::tlm_generic_payload& gp, sc_time& delay) {
        sc_dt::uint64 a = gp.get_address();
        unsigned len = gp.get_data_length();
        if (a + len > 64) {
            gp.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
            return;
        }
        if (gp.is_read()) memcpy(gp.get_data_ptr(), &mem[a], len);
        else memcpy(&mem[a], gp.get_data_ptr(), len);
        delay += sc_time(10, SC_NS);
        gp.set_response_status(tlm::TLM_OK_RESPONSE);
    }
    unsigned transport_dbg(tlm::tlm_generic_payload& gp) {
        sc_dt::uint64 a = gp.get_address();
        unsigned len = gp.get_data_length();
        memcpy(gp.get_data_ptr(), &mem[a], len);
        return len;
    }
};
struct cpu : sc_module {
    tlm_utils::simple_initiator_socket<cpu> sock;
    SC_CTOR(cpu) : sock("sock") { SC_THREAD(run); }
    void run() {
        tlm::tlm_generic_payload gp;
        sc_time d = SC_ZERO_TIME;
        unsigned int word = 0;
        gp.set_command(tlm::TLM_READ_COMMAND);
        gp.set_address(4);
        gp.set_data_ptr(reinterpret_cast<unsigned char*>(&word));
        gp.set_data_length(4);
        gp.set_streaming_width(4);
        gp.set_response_status(tlm::TLM_INCOMPLETE_RESPONSE);
        sock->b_transport(gp, d);
        wait(d);
        std::cout << sc_time_stamp() << " read word " << std::hex << word << std::dec
                  << " ok=" << gp.is_response_ok() << std::endl;
        word = 0xCAFEF00D;
        gp.set_command(tlm::TLM_WRITE_COMMAND);
        gp.set_address(32);
        d = SC_ZERO_TIME;
        sock->b_transport(gp, d);
        wait(d);
        unsigned char buf[8];
        memset(buf, 0, 8);
        gp.set_command(tlm::TLM_READ_COMMAND);
        gp.set_address(30);
        gp.set_data_ptr(buf);
        gp.set_data_length(8);
        unsigned n = sock->transport_dbg(gp);
        std::cout << sc_time_stamp() << " dbg " << n << ":";
        for (int i = 0; i < 8; i++) std::cout << " " << (int)buf[i];
        std::cout << std::endl;
        gp.set_address(60);
        gp.set_data_length(8);
        gp.set_response_status(tlm::TLM_INCOMPLETE_RESPONSE);
        d = SC_ZERO_TIME;
        sock->b_transport(gp, d);
        std::cout << "error=" << gp.is_response_error() << " " << gp.get_response_string() << std::endl;
    }
};
int sc_main(int, char*[]) {
    cpu c("c");
    ram r("r");
    c.sock.bind(r.sock);
    sc_start();
    return 0;
}
