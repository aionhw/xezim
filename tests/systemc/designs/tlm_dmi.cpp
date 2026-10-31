// TLM-2.0 direct memory interface (§11.2): get_direct_mem_ptr fills a
// tlm_dmi (pointer, range, access, latencies); the initiator then reads and
// writes the target's memory through the pointer; the target withdraws it
// with invalidate_direct_mem_ptr on the backward path.
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>

using namespace tlm;

SC_MODULE(target) {
    tlm_utils::simple_target_socket<target> socket;
    unsigned char mem[64];
    bool get_dmi(tlm_generic_payload& t, tlm_dmi& dmi) {
        std::cout << sc_time_stamp() << " dmi request at " << t.get_address() << std::endl;
        dmi.allow_read_write();
        dmi.set_dmi_ptr(mem);
        dmi.set_start_address(0);
        dmi.set_end_address(63);
        dmi.set_read_latency(sc_time(5, SC_NS));
        dmi.set_write_latency(sc_time(7, SC_NS));
        return true;
    }
    void b_transport(tlm_generic_payload& t, sc_time& d) {
        unsigned char* p = t.get_data_ptr();
        if (t.is_read()) for (unsigned i = 0; i < t.get_data_length(); i++) p[i] = mem[t.get_address() + i];
        else for (unsigned i = 0; i < t.get_data_length(); i++) mem[t.get_address() + i] = p[i];
        t.set_dmi_allowed(true);
        t.set_response_status(TLM_OK_RESPONSE);
        d += sc_time(20, SC_NS);
    }
    void revoke() {
        wait(100, SC_NS);
        std::cout << sc_time_stamp() << " invalidating 8..15, mem[9] = " << (int)mem[9] << std::endl;
        socket->invalidate_direct_mem_ptr(8, 15);
    }
    SC_CTOR(target) : socket("socket") {
        for (int i = 0; i < 64; i++) mem[i] = i;
        socket.register_b_transport(this, &target::b_transport);
        socket.register_get_direct_mem_ptr(this, &target::get_dmi);
        SC_THREAD(revoke);
    }
};

SC_MODULE(initiator) {
    tlm_utils::simple_initiator_socket<initiator> socket;
    bool dmi_valid;
    tlm_dmi dmi;
    void invalidate(sc_dt::uint64 s, sc_dt::uint64 e) {
        std::cout << sc_time_stamp() << " invalidated " << s << ".." << e << std::endl;
        dmi_valid = false;
    }
    void run() {
        tlm_generic_payload t;
        unsigned char b[4];
        t.set_command(TLM_READ_COMMAND);
        t.set_address(8);
        t.set_data_ptr(b);
        t.set_data_length(4);
        sc_time d = SC_ZERO_TIME;
        socket->b_transport(t, d);
        wait(d);
        std::cout << sc_time_stamp() << " read " << (int)b[0] << " dmi allowed " << t.is_dmi_allowed() << std::endl;
        dmi_valid = socket->get_direct_mem_ptr(t, dmi);
        std::cout << "dmi " << dmi_valid << " range " << dmi.get_start_address() << ".." << dmi.get_end_address()
                  << " rw " << dmi.is_read_write_allowed() << " latency " << dmi.get_read_latency() << std::endl;
        unsigned char* p = dmi.get_dmi_ptr();
        for (int k = 0; k < 3; k++) {
            if (!dmi_valid) break;
            p[9] = p[9] + 10;
            wait(dmi.get_write_latency());
            std::cout << sc_time_stamp() << " wrote mem[9] = " << (int)p[9] << std::endl;
            wait(40, SC_NS);
        }
        std::cout << sc_time_stamp() << " dmi valid " << dmi_valid << std::endl;
    }
    SC_CTOR(initiator) : socket("socket"), dmi_valid(false) {
        socket.register_invalidate_direct_mem_ptr(this, &initiator::invalidate);
        SC_THREAD(run);
    }
};

int sc_main(int, char*[]) {
    initiator i("i");
    target t("t");
    i.socket.bind(t.socket);
    sc_start();
    return 0;
}
