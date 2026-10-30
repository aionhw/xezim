// §8.3 sc_report_handler: actions per severity, per id and per (id,
// severity); suppress/force; stop_after; counts; verbosity; a user handler
// (set_handler) that calls the default one; reports from processes.
#include <systemc.h>

static int handled = 0;

void my_handler(const sc_report& r, const sc_actions& a) {
    handled++;
    std::cout << "[handler] " << r.get_msg_type() << " sev " << r.get_severity() << " msg '" << r.get_msg()
              << "' line " << r.get_line_number() << " at " << r.get_time() << " actions " << a << std::endl;
    sc_report_handler::default_handler(r, a & ~SC_STOP);
}

SC_MODULE(m) {
    void run() {
        for (int i = 0; i < 4; i++) {
            wait(1, SC_NS);
            SC_REPORT_WARNING("/m/warn", "from a process");
        }
        SC_REPORT_INFO_VERB("/m/info", "high", SC_HIGH);
        SC_REPORT_INFO_VERB("/m/info", "low", SC_LOW);
    }
    SC_CTOR(m) { SC_THREAD(run); }
};

int sc_main(int, char*[]) {
    SC_REPORT_INFO("/top/a", "plain info");
    SC_REPORT_WARNING("/top/a", "plain warning");
    sc_report_handler::set_actions(SC_WARNING, SC_DO_NOTHING);
    SC_REPORT_WARNING("/top/a", "silenced warning");
    sc_report_handler::set_actions("/top/b", SC_DISPLAY);
    SC_REPORT_WARNING("/top/b", "id rule wins over severity");
    sc_report_handler::set_actions("/top/b", SC_INFO, SC_DO_NOTHING);
    SC_REPORT_INFO("/top/b", "(id, severity) rule wins");
    SC_REPORT_ERROR("/top/b", "an error shown, not thrown");
    sc_actions old = sc_report_handler::suppress(SC_DISPLAY);
    SC_REPORT_ERROR("/top/b", "suppressed");
    sc_report_handler::suppress(old);
    std::cout << "counts: warning " << sc_report_handler::get_count(SC_WARNING) << " /top/b "
              << sc_report_handler::get_count("/top/b") << " /top/b error "
              << sc_report_handler::get_count("/top/b", SC_ERROR) << std::endl;
    std::cout << "new action ids " << sc_report_handler::get_new_action_id() << " "
              << sc_report_handler::get_new_action_id() << std::endl;
    sc_report_handler::set_actions(SC_WARNING, SC_DISPLAY);
    sc_report_handler::set_verbosity_level(SC_HIGH);
    sc_report_handler::set_handler(&my_handler);
    sc_report_handler::stop_after("/m/warn", 3);
    m u("u");
    sc_start(10, SC_NS);
    std::cout << "handled " << handled << " at " << sc_time_stamp() << std::endl;
    sc_report_handler::set_handler(&sc_report_handler::default_handler);
    sc_report_handler::set_actions(SC_ERROR, SC_DEFAULT_ERROR_ACTIONS);
    sc_report_handler::report(SC_ERROR, "/top/end", "thrown", "file.cpp", 7);
    std::cout << "not reached" << std::endl;
    return 0;
}
