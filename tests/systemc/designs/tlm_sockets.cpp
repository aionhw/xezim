// TLM-2.0 sockets (§13, §16): tagged sockets in arrays (a router whose
// callbacks get the socket's id), multi-passthrough sockets (several
// bindings, an id per binding), hierarchical binding (a parent's sockets
// bound to a child's), and tlm_initiator_socket/tlm_target_socket bound to
// a module that implements the interfaces itself (`bind(*this)`).
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>
#include <tlm_utils/multi_passthrough_initiator_socket.h>
#include <tlm_utils/multi_passthrough_target_socket.h>

using namespace tlm;

// A memory implementing the forward interface itself.
SC_MODULE(memory), public tlm_fw_transport_if<> {
    tlm_target_socket<> socket;
    int base;
    void b_transport(tlm_generic_payload& t, sc_time& d) {
        int* p = reinterpret_cast<int*>(t.get_data_ptr());
        *p = base + (int)t.get_address();
        d += sc_time(10, SC_NS);
        t.set_response_status(TLM_OK_RESPONSE);
        std::cout << sc_time_stamp() << " " << name() << " read " << t.get_address() << std::endl;
    }
    tlm_sync_enum nb_transport_fw(tlm_generic_payload&, tlm_phase&, sc_time&) { return TLM_COMPLETED; }
    bool get_direct_mem_ptr(tlm_generic_payload&, tlm_dmi&) { return false; }
    unsigned int transport_dbg(tlm_generic_payload& t) { return 0; }
    memory(sc_module_name n, int b) : sc_module(n), socket("socket"), base(b) { socket.bind(*this); }
};

// A wrapper whose target socket is its child memory's.
SC_MODULE(wrapper) {
    tlm_target_socket<> socket;
    memory mem;
    wrapper(sc_module_name n, int b) : sc_module(n), socket("socket"), mem("mem", b) { socket.bind(mem.socket); }
};

// A router: tagged target sockets (one per initiator) and initiator
// sockets (one per target) in arrays; the address picks the target.
SC_MODULE(router) {
    tlm_utils::simple_target_socket_tagged<router> tsock[2];
    tlm_utils::simple_initiator_socket_tagged<router> isock[2];
    void b_transport(int id, tlm_generic_payload& t, sc_time& d) {
        int port = (int)(t.get_address() >> 8) & 1;
        std::cout << sc_time_stamp() << " router from " << id << " to " << port << std::endl;
        t.set_address(t.get_address() & 0xff);
        isock[port]->b_transport(t, d);
    }
    SC_CTOR(router) {
        for (int i = 0; i < 2; i++) {
            tsock[i].register_b_transport(this, &router::b_transport, i);
        }
    }
};

// A hub with multi-passthrough sockets: callbacks get the binding index.
SC_MODULE(hub) {
    tlm_utils::multi_passthrough_target_socket<hub> tsock;
    tlm_utils::multi_passthrough_initiator_socket<hub> isock;
    void b_transport(int id, tlm_generic_payload& t, sc_time& d) {
        std::cout << sc_time_stamp() << " hub from " << id << " of " << tsock.size() << std::endl;
        isock[id % isock.size()]->b_transport(t, d);
    }
    SC_CTOR(hub) : tsock("tsock"), isock("isock") { tsock.register_b_transport(this, &hub::b_transport); }
};

// An initiator through a tlm_initiator_socket, implementing the backward
// interface itself.
SC_MODULE(cpu), public tlm_bw_transport_if<> {
    tlm_initiator_socket<> socket;
    int start;
    tlm_sync_enum nb_transport_bw(tlm_generic_payload&, tlm_phase&, sc_time&) { return TLM_COMPLETED; }
    void invalidate_direct_mem_ptr(sc_dt::uint64, sc_dt::uint64) {}
    void run() {
        wait(start, SC_NS);
        for (int i = 0; i < 2; i++) {
            tlm_generic_payload t;
            int v = 0;
            t.set_command(TLM_READ_COMMAND);
            t.set_address(i * 0x100 + 4 * start);
            t.set_data_ptr(reinterpret_cast<unsigned char*>(&v));
            t.set_data_length(4);
            sc_time d = SC_ZERO_TIME;
            socket->b_transport(t, d);
            wait(d);
            std::cout << sc_time_stamp() << " " << name() << " got " << v << std::endl;
        }
    }
    cpu(sc_module_name n, int s) : sc_module(n), socket("socket"), start(s) {
        socket.bind(*this);
        SC_THREAD(run);
    }
    SC_HAS_PROCESS(cpu);
};

SC_MODULE(top) {
    cpu c0, c1, c2;
    router r;
    hub h;
    memory m0;
    wrapper w1;
    memory m2, m3;
    top(sc_module_name n) : sc_module(n), c0("c0", 1), c1("c1", 2), c2("c2", 3), r("r"), h("h"),
                            m0("m0", 1000), w1("w1", 2000), m2("m2", 3000), m3("m3", 4000) {
        c0.socket.bind(r.tsock[0]);
        c1.socket.bind(r.tsock[1]);
        r.isock[0].bind(m0.socket);
        r.isock[1].bind(w1.socket);
        c2.socket.bind(h.tsock);
        h.isock.bind(m2.socket);
        h.isock.bind(m3.socket);
    }
};

int sc_main(int, char*[]) {
    top t("t");
    sc_start();
    return 0;
}
