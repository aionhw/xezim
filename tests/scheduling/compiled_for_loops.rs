//! Bytecode-compiled `for` loops in edge blocks (the For_init_vardecl /
//! For_step_other fallbacks were 83% of a customer run's wall time).
//! Register-backed loop vars, signal-backed `i++` steps, size-casts of the
//! loop var, and the still-AST self-reading counter shape.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn register_var_loop_with_cast_matches_model() {
    let src = r#"
module tb;
  logic clk = 0;
  logic [63:0] lanes [16];
  logic [63:0] src = 64'hdeadbeef01234567;
  logic [63:0] acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  initial for (int k = 0; k < 16; k++) lanes[k] = 0;
  always @(posedge clk) begin
    for (int i = 0; i < 16; i++) begin
      lanes[i] <= src ^ (64'(i) << 8) ^ acc;
    end
    acc <= acc + lanes[cyc & 15];
    cyc <= cyc + 1;
  end
  initial #62 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    // Rust mirror of the NBA semantics.
    let mut lanes = [0u64; 16];
    let mut acc: u64 = 0;
    let srcv: u64 = 0xdead_beef_0123_4567;
    let n = u(&sim, "cyc");
    for c in 0..n {
        let old = lanes;
        let old_acc = acc;
        for i in 0..16u64 {
            lanes[i as usize] = srcv ^ (i << 8) ^ old_acc;
        }
        acc = old_acc.wrapping_add(old[(c & 15) as usize]);
    }
    assert_eq!(u(&sim, "acc"), acc, "after {} cycles", n);
}

#[test]
fn signal_var_incr_step_loop() {
    let src = r#"
module tb;
  logic clk = 0;
  int i;
  logic [31:0] sum = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  always @(posedge clk) begin
    for (i = 0; i < 8; i++) sum <= sum + i; // last NBA wins: sum += 7
    cyc <= cyc + 1;
  end
  initial #22 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let n = u(&sim, "cyc");
    assert_eq!(u(&sim, "sum"), 7 * n, "one +7 per cycle (last NBA wins)");
}

#[test]
fn self_reading_counter_loop_still_correct() {
    // Excluded from register compilation (self-read gate) — must stay right.
    let src = r#"
module tb;
  logic clk = 0;
  logic [9:0] ptr [4];
  int cyc = 0;
  always #1 clk = ~clk;
  initial for (int k = 0; k < 4; k++) ptr[k] = 0;
  always @(posedge clk) begin
    for (int i = 0; i < 4; i++) ptr[i] <= ptr[i] + 1;
    cyc <= cyc + 1;
  end
  initial #22 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let n = u(&sim, "cyc");
    assert_eq!(u(&sim, "ptr[2]"), n, "each element counts every posedge");
}

#[test]
fn negative_bound_descending_loop_signed_compare() {
    // `i > -3` with a register-backed var: an unsigned step constant used
    // to strip the var's sign on the first i--, turning the compare
    // unsigned and exiting after one iteration.
    let src = r#"
module tb;
  logic clk = 0;
  logic signed [31:0] acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  always @(posedge clk) begin
    for (int i = 2; i > -3; i--) acc <= acc + i; // last NBA: acc + (-2)
    cyc <= cyc + 1;
  end
  initial #12 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let n = u(&sim, "cyc") as i64;
    let acc = u(&sim, "acc") as u32 as i32 as i64;
    assert_eq!(acc, -2 * n, "descending loop crosses zero with signed compare");
}

#[test]
fn loop_var_shadows_module_signal() {
    let src = r#"
module tb;
  logic clk = 0;
  int i = 777;
  logic [31:0] acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  always @(posedge clk) begin
    for (int i = 0; i < 5; i++) acc <= acc + i;
    cyc <= cyc + 1;
  end
  initial #12 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "i"), 777, "outer signal untouched by the loop var");
    let n = u(&sim, "cyc");
    assert_eq!(u(&sim, "acc"), 4 * n, "last NBA wins: acc + 4 per cycle");
}

#[test]
fn nested_register_var_loops_and_stride() {
    let src = r#"
module tb;
  logic clk = 0;
  logic [31:0] a = 0, b = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  always @(posedge clk) begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++)
        a <= a + i * 4 + j;          // last NBA: a + 15
    for (byte k = 0; k < 10; k += 2) b <= b + k; // last NBA: b + 8
    cyc <= cyc + 1;
  end
  initial #12 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let n = u(&sim, "cyc");
    assert_eq!(u(&sim, "a"), 15 * n);
    assert_eq!(u(&sim, "b"), 8 * n);
}
