// xezim_spike_dpi.cpp — Spike (riscv-isa-sim) shim exposing a small
// DPI-C surface for SystemVerilog testbenches running under xezim.
//
// Build:
//     make            # stub mode — no Spike dependency
//     make real       # real mode — links libriscv / libfesvr / libsoftfloat
//
// Use:
//     xezim ... --dpi-lib ./xezim_spike_dpi.so <sv files>

#include <cstdio>
#include <cstdint>
#include <cstring>
#include <string>
#include <memory>
#include <vector>
#include <utility>
#include <optional>

#if defined(XEZIM_SPIKE_REAL)
  // Pull in just enough of Spike's public surface for the minimum
  // "load ELF + step + read state" loop.
  #include <riscv/sim.h>
  #include <riscv/processor.h>
  #include <riscv/cfg.h>
  #include <riscv/devices.h>
  #include <riscv/debug_module.h>
#endif

namespace {

struct Shim {
    bool        initialised   = false;
    std::string isa           = "rv32imc";
    std::string priv          = "M";
    std::string elf;

    // Stub-mode bookkeeping for testing without a real CPU.
    uint64_t    stub_pc        = 0x80000000;
    uint64_t    stub_step_cnt  = 0;

#if defined(XEZIM_SPIKE_REAL)
    std::unique_ptr<cfg_t>    cfg;
    std::vector<std::pair<reg_t, abstract_mem_t*>> mems;  // owned via raw, released in finish()
    std::unique_ptr<sim_t>    sim;
    processor_t*              proc = nullptr;
    // Spike's default device map (CLINT @ 0x02000000, PLIC @ 0x0C000000)
    // collides with cv32e40p's anchored-at-0 layout. Anchor our RAM at
    // 0x80000000 (Spike convention) so the default devices fit below it.
    // Phase 3 can override via env vars once we add per-target maps.
    static constexpr uint64_t MEM_BASE = 0x80000000ull;
    static constexpr uint64_t MEM_SIZE = 0x10000000ull;  // 256 MiB
#endif
};

Shim* g_shim() {
    static Shim s;
    return &s;
}

} // namespace

extern "C" {

// Forward declarations so one entry point can call another.
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
    try {
        s->cfg = std::make_unique<cfg_t>();
        s->cfg->isa  = s->isa.c_str();
        s->cfg->priv = s->priv.c_str();
        s->cfg->mem_layout.clear();
        s->cfg->mem_layout.emplace_back(Shim::MEM_BASE, Shim::MEM_SIZE);
        s->cfg->hartids = {0};
        s->cfg->explicit_hartids = true;
        s->cfg->start_pc = 0; // typical cv32e40p reset vector lives at the ROM start

        auto* ram = new mem_t(Shim::MEM_SIZE);
        s->mems.emplace_back(Shim::MEM_BASE, ram);

        debug_module_config_t dm_cfg{};
        std::vector<std::string> args;
        if (!s->elf.empty()) args.push_back(s->elf);

        s->sim = std::make_unique<sim_t>(
            s->cfg.get(),
            /*halted=*/false,
            s->mems,
            /*plugin_device_factories=*/std::vector<device_factory_sargs_t>{},
            args,
            dm_cfg,
            /*log_path=*/nullptr,
            /*dtb_enabled=*/false,
            /*dtb_file=*/nullptr,
            /*socket_enabled=*/false,
            /*cmd_file=*/nullptr,
            /*instruction_limit=*/std::nullopt
        );
        s->proc = s->sim->get_core(static_cast<size_t>(0));
        s->initialised = (s->proc != nullptr);
        return s->initialised ? 0 : 1;
    } catch (const std::exception& e) {
        std::fprintf(stderr,
                     "[xezim_spike_dpi] init exception: %s\n", e.what());
        return 2;
    }
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
    if (!s->proc) return 0;
    const auto* state_before = s->proc->get_state();
    const uint64_t pc_before = state_before->pc;
    s->proc->step(1);
    const auto* state_after  = s->proc->get_state();
    if (retired_pc)   *retired_pc   = pc_before;
    if (retired_insn) *retired_insn = 0; // optional: re-fetch via MMU
    if (rd) *rd = -1;
    if (rd_val) *rd_val = 0;
    // Surface the most-recent commit-log entry (if Spike was built with
    // commit-log enabled). Without it, log_reg_write is empty and rd
    // stays -1; the SV side should treat -1 as "no register write".
    if (!state_after->log_reg_write.empty()) {
        const auto& it = state_after->log_reg_write.begin();
        const auto  encoded_addr = it->first;
        const auto  data = it->second;
        // Spike encodes (addr<<4)|(type) — type 0 = GPR.
        if ((encoded_addr & 0xF) == 0) {
            if (rd) *rd = static_cast<int>(encoded_addr >> 4);
            if (rd_val) *rd_val = static_cast<uint64_t>(data.v[0]);
        }
    }
    return 1;
#else
    if (retired_pc)   *retired_pc   = s->stub_pc;
    if (retired_insn) *retired_insn = 0x00100093u; // addi x1, x0, 1
    if (rd)           *rd           = 1;
    if (rd_val)       *rd_val       = ++s->stub_step_cnt;
    s->stub_pc += 4;
    return 1;
#endif
}

uint64_t xezim_spike_get_reg(int idx) {
    auto* s = g_shim();
    if (!s->initialised) return 0;
    if (idx == 32) return xezim_spike_get_pc();
#if defined(XEZIM_SPIKE_REAL)
    if (!s->proc) return 0;
    if (idx < 0 || idx >= NXPR) return 0;
    return static_cast<uint64_t>(s->proc->get_state()->XPR[idx]);
#else
    return (idx == 1) ? s->stub_step_cnt : 0;
#endif
}

uint64_t xezim_spike_get_pc(void) {
    auto* s = g_shim();
    if (!s->initialised) return 0;
#if defined(XEZIM_SPIKE_REAL)
    if (!s->proc) return 0;
    return static_cast<uint64_t>(s->proc->get_state()->pc);
#else
    return s->stub_pc;
#endif
}

void xezim_spike_finish(void) {
    auto* s = g_shim();
    if (!s->initialised) return;
    std::fprintf(stderr,
                 "[xezim_spike_dpi] finish (steps=%llu)\n",
                 (unsigned long long)s->stub_step_cnt);
#if defined(XEZIM_SPIKE_REAL)
    s->sim.reset();
    s->cfg.reset();
    for (auto& m : s->mems) delete m.second;
    s->mems.clear();
    s->proc = nullptr;
#endif
    s->initialised = false;
}

} // extern "C"
