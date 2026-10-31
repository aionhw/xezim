// TLM-2.0 non-blocking transport (§11, §15): nb_transport_fw/bw with the
// base protocol phases, through simple sockets; a target accepting the
// request at once (TLM_UPDATED with END_REQ) and answering later on the
// backward path; an initiator completing the response (TLM_COMPLETED); a
// target completing a transaction in one call (TLM_COMPLETED).
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <tlm_utils/simple_target_socket.h>

using namespace tlm;

SC_MODULE(initiator) {
    tlm_utils::simple_initiator_socket<initiator> socket;
    sc_event resp_done;
    int data;
    tlm_sync_enum nb_bw(tlm_generic_payload& t, tlm_phase& ph, sc_time& d) {
        std::cout << sc_time_stamp() << " initiator bw " << ph << " delay " << d << " data " << data << std::endl;
        if (ph == BEGIN_RESP) {
            resp_done.notify(d);
            ph = END_RESP;
            return TLM_COMPLETED;
        }
        return TLM_ACCEPTED;
    }
    void run() {
        tlm_generic_payload t;
        for (int i = 0; i < 3; i++) {
            data = 0;
            t.set_command(i == 2 ? TLM_WRITE_COMMAND : TLM_READ_COMMAND);
            t.set_address(i * 4);
            t.set_data_ptr(reinterpret_cast<unsigned char*>(&data));
            t.set_data_length(4);
            t.set_response_status(TLM_INCOMPLETE_RESPONSE);
            tlm_phase ph = BEGIN_REQ;
            sc_time d = sc_time(5, SC_NS);
            tlm_sync_enum r = socket->nb_transport_fw(t, ph, d);
            std::cout << sc_time_stamp() << " fw returned " << r << " phase " << ph << " delay " << d << std::endl;
            if (r == TLM_COMPLETED) {
                wait(d);
            } else {
                wait(resp_done);
            }
            std::cout << sc_time_stamp() << " done " << i << " " << t.get_response_string() << " data " << data << std::endl;
        }
    }
    SC_CTOR(initiator) : socket("socket"), data(0) {
        socket.register_nb_transport_bw(this, &initiator::nb_bw);
        SC_THREAD(run);
    }
};

SC_MODULE(target) {
    tlm_utils::simple_target_socket<target> socket;
    tlm_generic_payload* pending;
    sc_event respond;
    int mem[4];
    tlm_sync_enum nb_fw(tlm_generic_payload& t, tlm_phase& ph, sc_time& d) {
        std::cout << sc_time_stamp() << " target fw " << ph << " addr " << t.get_address() << " delay " << d << std::endl;
        int* p = reinterpret_cast<int*>(t.get_data_ptr());
        if (t.is_write()) {
            mem[t.get_address() / 4] = *p;
            t.set_response_status(TLM_OK_RESPONSE);
            d += sc_time(2, SC_NS);
            ph = END_RESP;
            return TLM_COMPLETED;
        }
        *p = mem[t.get_address() / 4];
        pending = &t;
        respond.notify(d + sc_time(10, SC_NS));
        ph = END_REQ;
        d = SC_ZERO_TIME;
        return TLM_UPDATED;
    }
    void responder() {
        while (true) {
            wait(respond);
            pending->set_response_status(TLM_OK_RESPONSE);
            tlm_phase ph = BEGIN_RESP;
            sc_time d = sc_time(1, SC_NS);
            tlm_sync_enum r = socket->nb_transport_bw(*pending, ph, d);
            std::cout << sc_time_stamp() << " bw returned " << r << " phase " << ph << std::endl;
        }
    }
    SC_CTOR(target) : socket("socket") {
        for (int i = 0; i < 4; i++) mem[i] = 100 + i;
        socket.register_nb_transport_fw(this, &target::nb_fw);
        SC_THREAD(responder);
    }
};

int sc_main(int, char*[]) {
    initiator i("i");
    target t("t");
    i.socket.bind(t.socket);
    sc_start();
    return 0;
}
