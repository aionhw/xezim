// Minimal smoke test for the Spike DPI shim.
//
// Build the .so and run:
//   make
//   xezim -s tb --dpi-lib ./xezim_spike_dpi.so test_spike_dpi.sv
//
// Expected stub-mode output:
//   [xezim_spike_dpi] init elf=/tmp/fake.elf isa=rv32imc priv=M
//   step 1: pc=0x80000000 insn=0x00100093 rd=1 val=1
//   step 2: pc=0x80000004 insn=0x00100093 rd=1 val=2
//   step 3: pc=0x80000008 insn=0x00100093 rd=1 val=3
//   x1 (via get_reg) = 3   pc (via get_pc) = 0x8000000c
//   [xezim_spike_dpi] finish (steps=3)

module tb;
  // DPI imports must be in module scope for xezim today; we include the
  // shared declarations here rather than at compilation-unit scope.
  `include "xezim_spike_dpi.svh"

  initial begin
    int            rc;
    longint unsigned pc;
    int unsigned     insn;
    int              rd;
    longint unsigned rd_val;
    int              retired;

    rc = xezim_spike_init("/tmp/fake.elf", "rv32imc", "M");
    if (rc != 0) begin
      $display("init failed (rc=%0d) — in stub mode this should be 0", rc);
      $finish;
    end

    for (int s = 1; s <= 3; s++) begin
      retired = xezim_spike_step(pc, insn, rd, rd_val);
      if (retired == 1) begin
        $display("step %0d: pc=0x%08x insn=0x%08x rd=%0d val=%0d",
                 s, pc, insn, rd, rd_val);
      end else begin
        $display("step %0d: did not retire", s);
      end
    end

    $display("x1 (via get_reg) = %0d   pc (via get_pc) = 0x%08x",
             xezim_spike_get_reg(1), xezim_spike_get_pc());

    xezim_spike_finish();
    $finish;
  end
endmodule
