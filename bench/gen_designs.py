#!/usr/bin/env python3
"""Generate the synthetic xezim benchmark designs (B2, B3, B5).

Each design is self-contained SystemVerilog with a fixed amount of work, so
that ns_per_insn / insns-per-second are comparable across machines. Work is
sized by parameters, never by wall-clock.
"""
import os, sys

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gen")
os.makedirs(OUT, exist_ok=True)


def write(name, text):
    p = os.path.join(OUT, name)
    with open(p, "w") as f:
        f.write(text)
    print(f"  {name}  ({len(text)} bytes)")


# ---------------------------------------------------------------- B2
def vm_dispatch(n_blocks=512, per_block=8, cycles=200_000):
    """Many small always_ff blocks; working set deliberately L2-resident.
    Isolates the bytecode interpreter's dispatch rate (indirect branches),
    with almost no memory pressure."""
    L = []
    L.append("// B2 vm-dispatch: interpreter dispatch rate, cache-resident.")
    L.append(f"// {n_blocks} blocks x {per_block} flops = {n_blocks*per_block} flops")
    L.append("module bench_vm_dispatch;")
    L.append("  bit clk = 0;")
    L.append("  int cyc = 0;")
    for b in range(n_blocks):
        L.append(f"  logic [7:0] s{b}_0, s{b}_1, s{b}_2, s{b}_3, s{b}_4, s{b}_5, s{b}_6, s{b}_7;")
    L.append("  always #1 clk = ~clk;")
    for b in range(n_blocks):
        L.append(f"  always_ff @(posedge clk) begin")
        L.append(f"    s{b}_0 <= s{b}_7 ^ 8'h5A;")
        for k in range(1, per_block):
            L.append(f"    s{b}_{k} <= s{b}_{k-1} + 8'd{(b + k) & 0xFF};")
        L.append(f"  end")
    L.append("  always_ff @(posedge clk) cyc <= cyc + 1;")
    L.append(f"  initial begin")
    L.append(f"    #({2*cycles});")
    L.append(f"    $display(\"BENCH_DONE cycles=%0d checksum=%0d\", cyc, s0_0 + s{n_blocks-1}_7);")
    L.append("    $finish;")
    L.append("  end")
    L.append("endmodule")
    write("b2_vm_dispatch.sv", "\n".join(L) + "\n")


def vm_dispatch_branchy(n_blocks=512, cycles=50_000):
    """B2b: same footprint as B2a, but the executed path is DATA-DEPENDENT.

    Each block dispatches on LFSR bits, so (a) a different arm of the case runs
    each cycle, giving the interpreter's `match` on the Insn variant an
    unpredictable target, and (b) each block's flops only move when its enable
    bit is set, so edge-skip gating fires a different SUBSET of blocks every
    cycle. B2a and B2b do the same amount of work and touch the same working
    set; the only difference is predictability, so (B2a - B2b) isolates the
    indirect-branch-predictor cost."""
    L = []
    L.append("// B2b vm-branchy: interpreter dispatch with an unpredictable path.")
    L.append(f"// {n_blocks} blocks, data-dependent case arms + per-block enables")
    L.append("module bench_vm_branchy;")
    L.append("  bit clk = 0;")
    L.append("  int cyc = 0;")
    L.append("  logic [31:0] lfsr = 32'hACE1_2345;")
    for b in range(n_blocks):
        L.append(f"  logic [7:0] s{b}_0, s{b}_1, s{b}_2, s{b}_3, s{b}_4, s{b}_5, s{b}_6, s{b}_7;")
    L.append("  always #1 clk = ~clk;")
    L.append("  // xorshift: a new, unpredictable control word every cycle")
    L.append("  always_ff @(posedge clk) begin")
    L.append("    lfsr <= lfsr ^ (lfsr << 13) ^ (lfsr >> 17) ^ (lfsr << 5);")
    L.append("    cyc  <= cyc + 1;")
    L.append("  end")
    for b in range(n_blocks):
        sel_lo = (b * 3) % 29
        en_bit = (b * 7) % 32
        L.append(f"  always_ff @(posedge clk) begin")
        L.append(f"    if (lfsr[{en_bit}]) begin")
        L.append(f"      case (lfsr[{sel_lo+2}:{sel_lo}])")
        L.append(f"        3'd0: s{b}_0 <= s{b}_7 + 8'd{(b + 1) & 0xFF};")
        L.append(f"        3'd1: s{b}_1 <= s{b}_0 ^ 8'h5A;")
        L.append(f"        3'd2: s{b}_2 <= s{b}_1 << 1;")
        L.append(f"        3'd3: s{b}_3 <= s{b}_2 >> 2;")
        L.append(f"        3'd4: s{b}_4 <= s{b}_3 & s{b}_6;")
        L.append(f"        3'd5: s{b}_5 <= s{b}_4 | 8'h0F;")
        L.append(f"        3'd6: s{b}_6 <= s{b}_5 - 8'd3;")
        L.append(f"        default: s{b}_7 <= s{b}_6 + s{b}_0;")
        L.append(f"      endcase")
        L.append(f"    end")
        L.append(f"  end")
    L.append(f"  initial begin")
    L.append(f"    #({2*cycles});")
    L.append(f"    $display(\"BENCH_DONE cycles=%0d checksum=%0d\", cyc, s0_0 + s{n_blocks-1}_7);")
    L.append("    $finish;")
    L.append("  end")
    L.append("endmodule")
    write("b2b_vm_branchy.sv", "\n".join(L) + "\n")


# ---------------------------------------------------------------- B3
def mem_sweep(log2_depth, cycles=100_000):
    """One memory of 2^log2_depth x 32b, accessed at LFSR-scattered addresses
    so the prefetcher cannot hide the latency. Sweeping log2_depth walks the
    working set from L1 through LLC into DRAM."""
    depth = 1 << log2_depth
    kib = depth * 4 // 1024
    L = []
    L.append(f"// B3 mem-sweep: working set = 2^{log2_depth} x 32b = {kib} KiB")
    L.append("module bench_mem_sweep;")
    L.append("  bit clk = 0;")
    L.append(f"  logic [31:0] mem [{depth}];")
    L.append("  logic [31:0] lfsr = 32'h1234_5678;")
    L.append("  logic [31:0] acc = 0;")
    L.append("  int cyc = 0;")
    L.append("  always #1 clk = ~clk;")
    L.append("  always_ff @(posedge clk) begin")
    L.append("    // xorshift keeps the address stream unpredictable")
    L.append("    lfsr <= lfsr ^ (lfsr << 13);")
    L.append(f"    acc  <= acc + mem[lfsr[{log2_depth-1}:0]];")
    L.append(f"    mem[(lfsr >> 7) & {depth-1}] <= acc ^ lfsr;")
    L.append("    cyc  <= cyc + 1;")
    L.append("  end")
    L.append(f"  initial begin")
    L.append(f"    #({2*cycles});")
    L.append("    $display(\"BENCH_DONE cycles=%0d checksum=%0d\", cyc, acc);")
    L.append("    $finish;")
    L.append("  end")
    L.append("endmodule")
    write(f"b3_mem_sweep_{log2_depth}.sv", "\n".join(L) + "\n")


# ---------------------------------------------------------------- B5
def constraint_rand(iters=20_000):
    """Randomization throughput: dist + foreach + unique + inline constraints.
    Branchy, allocation- and hash-heavy, and leans on the i128 exact arithmetic
    in the solver — which lowers very differently on aarch64 vs x86-64."""
    L = []
    L.append("// B5 constraint-rand: solver + PRNG throughput")
    L.append("module bench_constraint_rand;")
    L.append("  class Pkt;")
    L.append("    rand bit [7:0]  kind;")
    L.append("    rand bit [15:0] len;")
    L.append("    rand bit [7:0]  payload[];")
    L.append("    rand bit [3:0]  tags[4];")
    L.append("    constraint c_kind { kind dist {0 := 1, [1:8] :/ 6, 9 := 3}; }")
    L.append("    constraint c_len  { len inside {[64:512]}; len % 4 == 0; }")
    L.append("    constraint c_size { payload.size() == 8; }")
    L.append("    constraint c_elem { foreach (payload[i]) payload[i] inside {[1:200]}; }")
    L.append("    constraint c_uniq { unique {tags[0], tags[1], tags[2], tags[3]}; }")
    L.append("  endclass")
    L.append("  int ok = 0, fails = 0;")
    L.append("  initial begin")
    L.append("    Pkt p = new();")
    L.append(f"    repeat ({iters}) begin")
    L.append("      if (p.randomize() with { len > 128; }) ok++; else fails++;")
    L.append("    end")
    L.append("    $display(\"BENCH_DONE randomizations=%0d failures=%0d\", ok, fails);")
    L.append("    $finish;")
    L.append("  end")
    L.append("endmodule")
    write("b5_constraint_rand.sv", "\n".join(L) + "\n")


if __name__ == "__main__":
    print("generating benchmark designs into bench/gen/ ...")
    vm_dispatch()
    vm_dispatch_branchy()
    for n in (10, 12, 14, 16, 18, 20, 22):
        mem_sweep(n)
    constraint_rand()
    print("done.")
