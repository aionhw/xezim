//! Host-CPU benchmark workloads.
//!
//! Each workload is a deterministic, SELF-CHECKING design sized by a cycle
//! count, chosen so that different host-CPU characteristics dominate:
//!
//! - `comb_alu`     — deep combinational cone re-settled every edge
//!                    (single-thread integer ALU + dispatch)
//! - `wide_vec`     — 512-bit rotate/xor accumulator (wide-Value ops,
//!                    memory movement of multi-word values)
//! - `mem_array`    — pseudo-random walk over a 64 Ki-entry unpacked array
//!                    (cache/memory latency)
//! - `event_ctrl`   — event ping-pong between processes (scheduler,
//!                    branchy dispatch)
//! - `class_queue`  — class construction + queue/assoc traffic per cycle
//!                    (allocation + hashing)
//!
//! The design counts its own cycles (`cyc`); `check` reads it back and
//! compares every accumulator against a Rust mirror of the same arithmetic,
//! so a "fast but wrong" simulator change fails the bench instead of
//! producing an impressive number.

use crate::compiler::Simulator;

pub struct Workload {
    pub name: &'static str,
    /// What host characteristic this workload is meant to stress.
    pub stresses: &'static str,
    pub source: fn(cycles: u64) -> String,
    /// Simulation end time for the given cycle count.
    pub sim_time: fn(cycles: u64) -> u64,
    /// Panics with a description on mismatch.
    pub check: fn(sim: &Simulator),
}

fn sig(sim: &Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("bench signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("bench signal {} is x/z", n))
}

// ---------------------------------------------------------------- comb_alu

fn comb_alu_src(_cycles: u64) -> String {
    r#"
module tb;
  logic clk = 0;
  logic [7:0]  a = 8'd1, b = 8'd2;
  logic [15:0] s0, s1, s2, s3, prod;
  logic [31:0] acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  assign s0 = {8'd0, a ^ b};
  assign s1 = {8'd0, (a & 8'h0f) | (b & 8'hf0)};
  assign s2 = s0 + s1;
  always_comb begin
    prod = 16'h0;
    for (int i = 0; i < 8; i++)
      if (s2[i]) prod = prod + (s0 << i);
    s3 = prod ^ s2;
  end
  always_ff @(posedge clk) begin
    acc <= acc + {16'd0, s3};
    a   <= a + 8'd3;
    b   <= b + 8'd5;
    cyc <= cyc + 1;
  end
endmodule
"#
    .to_string()
}

fn comb_alu_check(sim: &Simulator) {
    let n = sig(sim, "cyc");
    let (mut a, mut b): (u8, u8) = (1, 2);
    let mut acc: u32 = 0;
    for _ in 0..n {
        let s0 = (a ^ b) as u16;
        let s1 = ((a & 0x0f) | (b & 0xf0)) as u16;
        let s2 = s0.wrapping_add(s1);
        let mut prod: u16 = 0;
        for i in 0..8 {
            if (s2 >> i) & 1 == 1 {
                prod = prod.wrapping_add(s0 << i);
            }
        }
        let s3 = prod ^ s2;
        acc = acc.wrapping_add(s3 as u32);
        a = a.wrapping_add(3);
        b = b.wrapping_add(5);
    }
    assert_eq!(sig(sim, "acc"), acc as u64, "comb_alu acc after {} cycles", n);
}

// ---------------------------------------------------------------- wide_vec

fn wide_vec_src(_cycles: u64) -> String {
    r#"
module tb;
  logic clk = 0;
  logic [511:0] w = 512'd1;
  logic [63:0]  fold = 0;
  logic [63:0]  x = 64'h9e3779b97f4a7c15;
  int cyc = 0;
  always #1 clk = ~clk;
  always_ff @(posedge clk) begin
    // rotate left by one 64-bit lane, xor the wrapped lane with x
    w    <= {w[447:0], w[511:448] ^ x};
    fold <= fold ^ w[63:0] ^ w[511:448];
    x    <= x + 64'h6a09e667f3bcc909;
    cyc  <= cyc + 1;
  end
endmodule
"#
    .to_string()
}

fn wide_vec_check(sim: &Simulator) {
    let n = sig(sim, "cyc");
    let mut lanes: [u64; 8] = [1, 0, 0, 0, 0, 0, 0, 0]; // lanes[0] = w[63:0]
    let mut fold: u64 = 0;
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..n {
        fold ^= lanes[0] ^ lanes[7];
        let wrapped = lanes[7] ^ x;
        // w <= {w[447:0], wrapped}: lanes shift up, wrapped becomes lane 0
        for i in (1..8).rev() {
            lanes[i] = lanes[i - 1];
        }
        lanes[0] = wrapped;
        x = x.wrapping_add(0x6a09_e667_f3bc_c909);
    }
    assert_eq!(sig(sim, "fold"), fold, "wide_vec fold after {} cycles", n);
}

// ---------------------------------------------------------------- mem_array

fn mem_array_src(_cycles: u64) -> String {
    r#"
module tb;
  logic clk = 0;
  int mem [65536];
  int idx = 0;
  int acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  initial for (int i = 0; i < 65536; i++) mem[i] = i;
  always_ff @(posedge clk) begin
    idx      <= (idx * 5 + 1) & 32'h0000_ffff;
    mem[idx] <= mem[idx] + idx;
    acc      <= acc ^ mem[idx];
    cyc      <= cyc + 1;
  end
endmodule
"#
    .to_string()
}

fn mem_array_check(sim: &Simulator) {
    let n = sig(sim, "cyc");
    let mut mem: Vec<u32> = (0..65536u32).collect();
    let mut idx: u32 = 0;
    let mut acc: u32 = 0;
    for _ in 0..n {
        // NBA semantics: all three reads see pre-edge state
        let cur = mem[idx as usize];
        acc ^= cur;
        mem[idx as usize] = cur.wrapping_add(idx);
        idx = (idx.wrapping_mul(5).wrapping_add(1)) & 0xffff;
    }
    assert_eq!(sig(sim, "acc") as u32, acc, "mem_array acc after {} cycles", n);
}

// ---------------------------------------------------------------- event_ctrl

fn event_ctrl_src(cycles: u64) -> String {
    format!(
        r#"
module tb;
  event ping, pong;
  int hops = 0;
  int cyc = 0;
  initial begin
    for (cyc = 0; cyc < {cycles}; cyc++) begin
      #1 ->ping;
      @(pong);
    end
  end
  initial forever begin
    @(ping);
    hops = hops + 1;
    ->pong;
  end
endmodule
"#
    )
}

fn event_ctrl_check(sim: &Simulator) {
    let n = sig(sim, "cyc");
    assert_eq!(sig(sim, "hops"), n, "event_ctrl one hop per iteration");
}

// ---------------------------------------------------------------- class_queue

fn class_queue_src(cycles: u64) -> String {
    format!(
        r#"
class Item;
  int v;
  function new(int x); v = x; endfunction
endclass
module tb;
  Item q[$];
  int hist[int];
  int acc = 0;
  int cyc = 0;
  initial begin
    for (cyc = 0; cyc < {cycles}; cyc++) begin
      Item it = new(cyc);
      q.push_back(it);
      if (q.size() > 8) begin
        Item head = q.pop_front();
        acc = acc ^ head.v;
      end
      hist[cyc % 17] = hist[cyc % 17] + 1;
      #1;
    end
  end
endmodule
"#
    )
}

fn class_queue_check(sim: &Simulator) {
    let n = sig(sim, "cyc");
    let mut acc: u32 = 0;
    let mut size = 0u64;
    let mut head = 0u64;
    for i in 0..n {
        size += 1;
        if size > 8 {
            acc ^= head as u32;
            head += 1;
            size -= 1;
        }
        let _ = i;
    }
    assert_eq!(sig(sim, "acc") as u32, acc, "class_queue acc after {} cycles", n);
    let h0 = sig(sim, "hist[0]");
    assert_eq!(h0, n.div_ceil(17), "class_queue hist[0]");
}

// ----------------------------------------------------------------

pub fn workloads() -> Vec<Workload> {
    vec![
        Workload {
            name: "comb_alu",
            stresses: "integer ALU / comb settle",
            source: comb_alu_src,
            sim_time: |c| c * 2 + 2,
            check: comb_alu_check,
        },
        Workload {
            name: "wide_vec",
            stresses: "512-bit values / memory move",
            source: wide_vec_src,
            sim_time: |c| c * 2 + 2,
            check: wide_vec_check,
        },
        Workload {
            name: "mem_array",
            stresses: "cache / memory latency",
            source: mem_array_src,
            sim_time: |c| c * 2 + 2,
            check: mem_array_check,
        },
        Workload {
            name: "event_ctrl",
            stresses: "scheduler / event dispatch",
            source: event_ctrl_src,
            sim_time: |c| c + 4,
            check: event_ctrl_check,
        },
        Workload {
            name: "class_queue",
            stresses: "allocation / hashing",
            source: class_queue_src,
            sim_time: |c| c + 4,
            check: class_queue_check,
        },
    ]
}
