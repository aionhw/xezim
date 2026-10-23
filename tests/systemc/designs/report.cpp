#include <systemc.h>
SC_MODULE(m) {
    void run() {
        SC_REPORT_INFO("tag", "hello info");
        wait(5, SC_NS);
        SC_REPORT_WARNING("wtag", "careful");
        wait(5, SC_NS);
        std::cout << "stopping at " << sc_time_stamp() << std::endl;
        sc_stop();
        wait(5, SC_NS);
        std::cout << "not reached" << std::endl;
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m u("u");
    sc_start();
    std::cout << "back in sc_main at " << sc_time_stamp() << std::endl;
    SC_REPORT_ERROR("etag", "an error");
    std::cout << "after error" << std::endl;
    return 0;
}
