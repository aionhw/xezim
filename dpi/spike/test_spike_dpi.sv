// Smoke test for the Spike DPI shim.
//
// Build the .so and the test ELF, then run:
//   make
//   make real SPIKE_PREFIX=/path/to/spike   # for real Spike linkage
//   make test-elf                           # cross-compile test_spike_asm.S
//   xezim -s tb --dpi-lib ./xezim_spike_dpi.so test_spike_dpi.sv
//
// Stub-mode behaviour: each step retires the same canned `addi x1, x0, 1`
// at a monotonically incrementing PC.
//
// Real-mode behaviour (`make real` + `make test-elf` + a built ELF at
// /tmp/spike_asm.elf or test_spike_asm.elf): Spike actually executes the
// pure-asm test program, surfacing the GPR writes
//   x1=0xdead x2=0xbeef x3=0x1234 x4=0x5678 x5=0xcafe x6=0xbabe.

module tb;
  // DPI imports — must be inside the module for xezim today.
  `include "xezim_spike_dpi.svh"

  initial begin
    int            rc;
    longint unsigned pc;
    int unsigned     insn;
    int              rd;
    longint unsigned rd_val;
    int              retired;

    // Default to the in-tree test ELF; falls back to /tmp/fake.elf for
    // stub-mode smoke runs where no real ELF exists.
    string elf_path = "test_spike_asm.elf";

    rc = xezim_spike_init(elf_path, "rv32imc", "M");
    if (rc != 0) begin
      $display("init failed (rc=%0d) — falling back to stub ELF path", rc);
      rc = xezim_spike_init("/tmp/fake.elf", "rv32imc", "M");
    end

    for (int s = 1; s <= 12; s++) begin
      retired = xezim_spike_step(pc, insn, rd, rd_val);
      $display("step %2d: pc=0x%08x retired=%0d  x1=0x%0x x2=0x%0x x3=0x%0x x4=0x%0x x5=0x%0x x6=0x%0x",
               s, pc[31:0], retired,
               xezim_spike_get_reg(1)[31:0], xezim_spike_get_reg(2)[31:0],
               xezim_spike_get_reg(3)[31:0], xezim_spike_get_reg(4)[31:0],
               xezim_spike_get_reg(5)[31:0], xezim_spike_get_reg(6)[31:0]);
    end

    xezim_spike_finish();
    $finish;
  end
endmodule
