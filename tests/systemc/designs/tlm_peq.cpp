// TLM-2.0 utilities (§16.3–§16.4): peq_with_get (transactions come out at
// their notified times, through its event and get_next_transaction) and
// peq_with_cb_and_phase (a callback with the phase at the notified time);
// tlm_quantumkeeper with the global quantum (§12.2): local time, sync when
// the quantum is used up.
#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/peq_with_get.h>
#include <tlm_utils/peq_with_cb_and_phase.h>
#include <tlm_utils/tlm_quantumkeeper.h>

using namespace tlm;

SC_MODULE(m) {
    tlm_utils::peq_with_get<tlm_generic_payload> peq;
    tlm_utils::peq_with_cb_and_phase<m> cbpeq;
    tlm_generic_payload t[3];
    tlm_utils::tlm_quantumkeeper qk;
    void cb(tlm_generic_payload& p, const tlm_phase& ph) {
        std::cout << sc_time_stamp() << " callback addr " << p.get_address() << " " << ph << std::endl;
    }
    void producer() {
        for (int i = 0; i < 3; i++) t[i].set_address(10 * i);
        peq.notify(t[0], sc_time(5, SC_NS));
        peq.notify(t[1], sc_time(2, SC_NS));
        peq.notify(t[2], sc_time(5, SC_NS));
        tlm_phase ph = BEGIN_REQ;
        cbpeq.notify(t[1], ph, sc_time(3, SC_NS));
        ph = END_RESP;
        cbpeq.notify(t[2], ph, sc_time(1, SC_NS));
        wait(21, SC_NS);
        ph = BEGIN_RESP;
        cbpeq.notify(t[0], ph);
    }
    void consumer() {
        while (true) {
            wait(peq.get_event());
            tlm_generic_payload* p;
            while ((p = peq.get_next_transaction()) != 0)
                std::cout << sc_time_stamp() << " peq got addr " << p->get_address() << std::endl;
        }
    }
    void decoupled() {
        qk.reset();
        for (int i = 0; i < 6; i++) {
            qk.inc(sc_time(4, SC_NS));
            std::cout << sc_time_stamp() << " local " << qk.get_local_time() << " current " << qk.get_current_time()
                      << " need sync " << qk.need_sync() << std::endl;
            if (qk.need_sync()) qk.sync();
        }
    }
    SC_CTOR(m) : peq("peq"), cbpeq(this, &m::cb) {
        tlm_global_quantum::instance().set(sc_time(10, SC_NS));
        qk.reset();
        SC_THREAD(producer);
        SC_THREAD(consumer);
        SC_THREAD(decoupled);
    }
};

int sc_main(int, char*[]) {
    m u("u");
    std::cout << "quantum " << tlm_global_quantum::instance().get() << std::endl;
    sc_start(100, SC_NS);
    return 0;
}
