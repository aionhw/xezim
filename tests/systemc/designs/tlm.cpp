// t5: TLM-2.0 loosely-timed write/read through simple sockets
#include <systemc>
#include <tlm>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>
#include <cstring>
#include <iostream>
using namespace sc_core;

struct Memory : sc_module {
    tlm_utils::simple_target_socket<Memory> socket;
    unsigned char mem[64];
    SC_CTOR(Memory) : socket("socket") {
        std::memset(mem, 0, sizeof(mem));
        socket.register_b_transport(this, &Memory::b_transport);
    }
    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
        sc_dt::uint64 a = trans.get_address();
        unsigned len = trans.get_data_length();
        if (a + len > sizeof(mem)) {
            trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
            return;
        }
        if (trans.is_write()) std::memcpy(&mem[a], trans.get_data_ptr(), len);
        else std::memcpy(trans.get_data_ptr(), &mem[a], len);
        delay += sc_time(5, SC_NS);
        trans.set_response_status(tlm::TLM_OK_RESPONSE);
    }
};

struct Initiator : sc_module {
    tlm_utils::simple_initiator_socket<Initiator> socket;
    SC_CTOR(Initiator) : socket("socket") { SC_THREAD(run); }
    bool access(tlm::tlm_command cmd, unsigned addr, unsigned& data) {
        tlm::tlm_generic_payload t;
        sc_time delay = SC_ZERO_TIME;
        t.set_command(cmd);
        t.set_address(addr);
        t.set_data_ptr(reinterpret_cast<unsigned char*>(&data));
        t.set_data_length(4);
        t.set_streaming_width(4);
        t.set_response_status(tlm::TLM_INCOMPLETE_RESPONSE);
        socket->b_transport(t, delay);
        wait(delay);
        return t.is_response_ok();
    }
    void run() {
        unsigned w = 0xCAFE, r = 0, bad = 0;
        access(tlm::TLM_WRITE_COMMAND, 8, w);
        access(tlm::TLM_READ_COMMAND, 8, r);
        bool ok = access(tlm::TLM_READ_COMMAND, 100, bad);
        std::cout << std::hex << "read 0x" << r << std::dec << " at " << sc_time_stamp()
                  << ", out-of-range ok=" << ok << std::endl;
    }
};

SC_MODULE(Top) {
    Initiator init{"init"};
    Memory mem{"mem"};
    SC_CTOR(Top) { init.socket.bind(mem.socket); }
};

int sc_main(int, char*[]) {
    Top top("top");
    sc_start();
    return 0;
}
