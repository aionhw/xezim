// xezim_spike_dpi.cpp — Spike (riscv-isa-sim) shim exposing a small
// DPI-C surface for SystemVerilog testbenches running under xezim.
//
// Build:
//     make            # produces ./xezim_spike_dpi.so
//
// Use:
//     xezim ... --dpi-lib ./xezim_spike_dpi.so <sv files>
//
// SV side: include xezim_spike_dpi.svh and call the imports.
//
// Phases:
//   Phase 1 (this file)  : stub mode — calls print and return canned
//                          values so the SV / xezim integration can be
//                          tested standalone. Compile-time switch
//                          XEZIM_SPIKE_REAL=1 turns on the real path.
//   Phase 2 (next)       : the marked TODO blocks become real sim_t /
//                          processor_t calls into libriscv.so.

#include <cstdio>
#include <cstdint>
#include <cstring>
#include <string>
#include <memory>

#if defined(XEZIM_SPIKE_REAL)
//   #include <riscv/sim.h>
//   #include <riscv/processor.h>
//   #include <fesvr/htif.h>
// Including the real Spike headers pulls in a wider toolchain dep, so
// we only require them when XEZIM_SPIKE_REAL is on.
#endif

namespace {

struct Shim {
    bool initialised = false;
    std::string isa;
    std::string priv;
    std::string elf;

    // Stub-mode bookkeeping so xezim_spike_step() returns *something*
    // testable without a real CPU running.
    uint64_t stub_pc       = 0x80000000;   // typical entry point
    uint64_t stub_step_cnt = 0;

#if defined(XEZIM_SPIKE_REAL)
    // std::unique_ptr<sim_t> sim;
    // processor_t*           proc = nullptr;
#endif
};

Shim* g_shim() {
    static Shim s;
    return &s;
}

} // namespace

extern "C" {

// Forward declarations so the body of one C entry point can call another
// (xezim_spike_get_reg(32) delegates to xezim_spike_get_pc).
int      xezim_spike_init(const char*, const char*, const char*);
int      xezim_spike_step(uint64_t*, uint32_t*, int*, uint64_t*);
uint64_t xezim_spike_get_reg(int);
uint64_t xezim_spike_get_pc(void);
void     xezim_spike_finish(void);

int xezim_spike_init(const char* elf_path, const char* isa, const char* priv) {
    auto* s = g_shim();
    if (s->initialised) {
        std::fprintf(stderr,
                     "[xezim_spike_dpi] warning: already initialised\n");
        return 0;
    }
    s->elf  = elf_path ? elf_path : "";
    s->isa  = isa      ? isa      : "rv32imc";
    s->priv = priv     ? priv     : "M";
    std::fprintf(stderr,
                 "[xezim_spike_dpi] init elf=%s isa=%s priv=%s\n",
                 s->elf.c_str(), s->isa.c_str(), s->priv.c_str());

#if defined(XEZIM_SPIKE_REAL)
    // TODO: construct cfg_t from isa/priv, build memory map matching
    // cv32e40p (ROM @ 0x0000_0000, RAM @ 0x0000_0000+ROM_SIZE), load
    // the ELF via htif_hexwriter / direct memif writes, create sim_t,
    // grab proc = sim->get_core(0).
    return 1; // not yet implemented
#else
    s->initialised   = true;
    s->stub_pc       = 0x80000000;
    s->stub_step_cnt = 0;
    return 0;
#endif
}

int xezim_spike_step(uint64_t* retired_pc,
                     uint32_t* retired_insn,
                     int*      rd,
                     uint64_t* rd_val) {
    auto* s = g_shim();
    if (!s->initialised) {
        return 0;
    }

#if defined(XEZIM_SPIKE_REAL)
    // s->proc->step(1);
    // const auto& last = s->proc->get_state()->log_reg_write;
    // populate *retired_pc / *retired_insn / *rd / *rd_val from Spike
    // state and return 1 (one instruction retired this step).
    return 0;
#else
    // Stub: pretend we executed a `addi x1, x0, 1` at the current PC.
    if (retired_pc)   *retired_pc   = s->stub_pc;
    if (retired_insn) *retired_insn = 0x00100093u; // addi x1, x0, 1
    if (rd)           *rd           = 1;
    if (rd_val)       *rd_val       = static_cast<int64_t>(++s->stub_step_cnt);
    s->stub_pc += 4;
    return 1;
#endif
}

uint64_t xezim_spike_get_reg(int idx) {
    auto* s = g_shim();
    if (!s->initialised) {
        return 0;
    }
    if (idx == 32) {
        return xezim_spike_get_pc();
    }
#if defined(XEZIM_SPIKE_REAL)
    // return s->proc->get_state()->XPR[idx];
    return 0;
#else
    // Stub mirrors what step() pretends to write.
    return (idx == 1) ? s->stub_step_cnt : 0;
#endif
}

uint64_t xezim_spike_get_pc(void) {
    auto* s = g_shim();
    if (!s->initialised) {
        return 0;
    }
#if defined(XEZIM_SPIKE_REAL)
    // return s->proc->get_state()->pc;
    return 0;
#else
    return s->stub_pc;
#endif
}

void xezim_spike_finish(void) {
    auto* s = g_shim();
    if (!s->initialised) {
        return;
    }
    std::fprintf(stderr,
                 "[xezim_spike_dpi] finish (steps=%llu)\n",
                 (unsigned long long)s->stub_step_cnt);
#if defined(XEZIM_SPIKE_REAL)
    // s->sim.reset();
    // s->proc = nullptr;
#endif
    s->initialised = false;
}

} // extern "C"
