//! Simulation tests for the SystemVerilog compiler/simulator.

use xezim::{simulate, simulate_multi};
use xezim::compiler::Value;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn sim_ok(src: &str) -> xezim::compiler::Simulator {
    match simulate(src, 100_000) {
        Ok(sim) => sim,
        Err(e) => panic!("Simulation failed: {}", e),
    }
}

fn sim_ok_plusargs(src: &str, plusargs: &[&str]) -> xezim::compiler::Simulator {
    let source = src.to_string();
    let plusargs_vec: Vec<String> = plusargs.iter().map(|s| (*s).to_string()).collect();
    match simulate_multi(
        &[source],
        100_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        false,
        &plusargs_vec,
        1,
    ) {
        Ok(sim) => sim,
        Err(e) => panic!("Simulation failed: {}", e),
    }
}

fn temp_file_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut p = std::env::temp_dir();
    p.push(format!("xezim_{}_{}_{}", tag, std::process::id(), nanos));
    p
}

#[test]
fn test_sim_test_plusargs() {
    let sim = sim_ok_plusargs(
        "
        module test;
            int a, b;
            initial begin
                a = $test$plusargs(\"FOO\");
                b = $test$plusargs(\"BAR\");
                $display(\"a=%0d b=%0d\", a, b);
                $finish;
            end
        endmodule
        ",
        &["+FOO", "+N=42"],
    );
    assert!(sim.output[0].message.contains("a=1 b=0"));
}

#[test]
fn test_sim_value_plusargs() {
    let sim = sim_ok_plusargs(
        "
        module test;
            int n;
            initial begin
                n = 0;
                if ($value$plusargs(\"N=%0d\", n))
                    $display(\"n=%0d\", n);
                else
                    $display(\"n=miss\");
                $finish;
            end
        endmodule
        ",
        &["+N=123"],
    );
    assert!(sim.output[0].message.contains("n=123"));
}

#[test]
fn test_sim_readmemh_loads_array_data() {
    let mem_path = temp_file_path("readmemh.hex");
    let out_path = temp_file_path("readmemh.out");
    fs::write(&mem_path, "01\nab\n0f\n").expect("write mem file");
    let mem_path_sv = mem_path.to_string_lossy().replace('\\', "\\\\");
    let out_path_sv = out_path.to_string_lossy().replace('\\', "\\\\");
    let src = format!(
        "
        module test;
            reg [7:0] mem [0:3];
            integer fd;
            initial begin
                $readmemh(\"{}\", mem);
                fd = $fopen(\"{}\", \"w\");
                $fwrite(fd, \"%02h %02h %02h\\n\", mem[0], mem[1], mem[2]);
                $fclose(fd);
                $finish;
            end
        endmodule
        ",
        mem_path_sv,
        out_path_sv
    );
    let _sim = sim_ok(&src);
    let contents = fs::read_to_string(&out_path).expect("read readmemh output");
    let _ = fs::remove_file(&mem_path);
    let _ = fs::remove_file(&out_path);
    assert_eq!(contents, "01 ab 0f\n");
}

#[test]
fn test_sim_fopen_fwrite_fclose_writes_file() {
    let out_path = temp_file_path("fwrite.out");
    let out_path_sv = out_path.to_string_lossy().replace('\\', "\\\\");
    let src = format!(
        "
        module test;
            integer fd;
            initial begin
                fd = $fopen(\"{}\", \"w\");
                $fwrite(fd, \"hello %0d\\n\", 42);
                $fclose(fd);
                $finish;
            end
        endmodule
        ",
        out_path_sv
    );
    let _sim = sim_ok(&src);
    let contents = fs::read_to_string(&out_path).expect("read fwrite output");
    let _ = fs::remove_file(&out_path);
    assert_eq!(contents, "hello 42\n");
}

#[test]
fn test_value_arithmetic() {
    let a = Value::from_u64(10, 32);
    let b = Value::from_u64(3, 32);
    assert_eq!(a.add(&b).to_u64(), Some(13));
    assert_eq!(a.sub(&b).to_u64(), Some(7));
    assert_eq!(a.mul(&b).to_u64(), Some(30));
}

#[test]
fn test_value_bitwise() {
    let a = Value::from_u64(0b1100, 4);
    let b = Value::from_u64(0b1010, 4);
    assert_eq!(a.bitwise_and(&b).to_u64(), Some(0b1000));
    assert_eq!(a.bitwise_or(&b).to_u64(), Some(0b1110));
    assert_eq!(a.bitwise_xor(&b).to_u64(), Some(0b0110));
}

#[test]
fn test_sim_assign_and() {
    let sim = sim_ok("
        module test;
            logic a, b, y;
            assign y = a & b;
            initial begin
                a = 1; b = 1; #1;
                $display(\"y = %b\", y);
                a = 1; b = 0; #1;
                $display(\"y = %b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("y = 1"));
    assert!(sim.output[1].message.contains("y = 0"));
}

#[test]
fn test_sim_assign_or() {
    let sim = sim_ok("
        module test;
            logic a, b, y;
            assign y = a | b;
            initial begin
                a = 0; b = 0; #1; $display(\"%b\", y);
                a = 1; b = 0; #1; $display(\"%b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("0"));
    assert!(sim.output[1].message.contains("1"));
}

#[test]
fn test_sim_assign_xor() {
    let sim = sim_ok("
        module test;
            logic a, b, y;
            assign y = a ^ b;
            initial begin
                a = 1; b = 1; #1; $display(\"%b\", y);
                a = 1; b = 0; #1; $display(\"%b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("0"));
    assert!(sim.output[1].message.contains("1"));
}

#[test]
fn test_sim_not() {
    let sim = sim_ok("
        module test;
            logic a, y;
            assign y = ~a;
            initial begin
                a = 0; #1; $display(\"%b\", y);
                a = 1; #1; $display(\"%b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("1"));
    assert!(sim.output[1].message.contains("0"));
}

#[test]
fn test_sim_multibit_add() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, b, sum;
            assign sum = a + b;
            initial begin
                a = 100; b = 55; #1;
                $display(\"sum=%d\", sum);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("sum=155"));
}

#[test]
fn test_sim_ternary_mux() {
    let sim = sim_ok("
        module test;
            logic sel;
            logic [7:0] a, b, y;
            assign y = sel ? a : b;
            initial begin
                a = 42; b = 99;
                sel = 0; #1; $display(\"%d\", y);
                sel = 1; #1; $display(\"%d\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("99"));
    assert!(sim.output[1].message.contains("42"));
}

#[test]
fn test_sim_concatenation() {
    let sim = sim_ok("
        module test;
            logic [3:0] hi, lo;
            logic [7:0] out;
            assign out = {hi, lo};
            initial begin
                hi = 4'hA; lo = 4'h5; #1;
                $display(\"%h\", out);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("a5"));
}

#[test]
fn test_sim_always_comb_case_mux() {
    let sim = sim_ok("
        module test;
            logic [1:0] sel;
            logic [7:0] a, b, c, d, y;
            always_comb begin
                case (sel)
                    2'b00: y = a;
                    2'b01: y = b;
                    2'b10: y = c;
                    default: y = d;
                endcase
            end
            initial begin
                a = 10; b = 20; c = 30; d = 40;
                sel = 0; #1; $display(\"y=%d\", y);
                sel = 1; #1; $display(\"y=%d\", y);
                sel = 2; #1; $display(\"y=%d\", y);
                sel = 3; #1; $display(\"y=%d\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("y=10"));
    assert!(sim.output[1].message.contains("y=20"));
    assert!(sim.output[2].message.contains("y=30"));
    assert!(sim.output[3].message.contains("y=40"));
}

#[test]
fn test_sim_always_comb_if_else() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, b, max_val;
            always_comb begin
                if (a > b) max_val = a;
                else max_val = b;
            end
            initial begin
                a = 50; b = 30; #1; $display(\"max=%d\", max_val);
                a = 10; b = 80; #1; $display(\"max=%d\", max_val);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("max=50"));
    assert!(sim.output[1].message.contains("max=80"));
}

#[test]
fn test_sim_chained_assign() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, b, c;
            assign b = a + 1;
            assign c = b * 2;
            initial begin
                a = 5; #1;
                $display(\"a=%d b=%d c=%d\", a, b, c);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("a=5"));
    assert!(sim.output[0].message.contains("b=6"));
    assert!(sim.output[0].message.contains("c=12"));
}

#[test]
fn test_sim_full_adder() {
    let sim = sim_ok("
        module test;
            logic a, b, cin, sum, cout;
            assign sum = a ^ b ^ cin;
            assign cout = (a & b) | (cin & (a ^ b));
            initial begin
                a=0; b=0; cin=0; #1; $display(\"%b%b%b -> s=%b c=%b\", a, b, cin, sum, cout);
                a=0; b=1; cin=0; #1; $display(\"%b%b%b -> s=%b c=%b\", a, b, cin, sum, cout);
                a=1; b=1; cin=0; #1; $display(\"%b%b%b -> s=%b c=%b\", a, b, cin, sum, cout);
                a=1; b=1; cin=1; #1; $display(\"%b%b%b -> s=%b c=%b\", a, b, cin, sum, cout);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("s=0") && sim.output[0].message.contains("c=0"));
    assert!(sim.output[1].message.contains("s=1") && sim.output[1].message.contains("c=0"));
    assert!(sim.output[2].message.contains("s=0") && sim.output[2].message.contains("c=1"));
    assert!(sim.output[3].message.contains("s=1") && sim.output[3].message.contains("c=1"));
}

#[test]
fn test_sim_decoder_2to4() {
    let sim = sim_ok("
        module test;
            logic [1:0] in_val;
            logic [3:0] out_val;
            assign out_val = 4'b0001 << in_val;
            initial begin
                in_val = 0; #1; $display(\"in=%d out=%b\", in_val, out_val);
                in_val = 1; #1; $display(\"in=%d out=%b\", in_val, out_val);
                in_val = 2; #1; $display(\"in=%d out=%b\", in_val, out_val);
                in_val = 3; #1; $display(\"in=%d out=%b\", in_val, out_val);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("out=0001"));
    assert!(sim.output[1].message.contains("out=0010"));
    assert!(sim.output[2].message.contains("out=0100"));
    assert!(sim.output[3].message.contains("out=1000"));
}

#[test]
fn test_sim_comparison_ops() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, b;
            logic eq_r, neq_r, lt_r, gt_r, leq_r, geq_r;
            assign eq_r  = (a == b);
            assign neq_r = (a != b);
            assign lt_r  = (a < b);
            assign gt_r  = (a > b);
            assign leq_r = (a <= b);
            assign geq_r = (a >= b);
            initial begin
                a = 10; b = 20; #1;
                $display(\"eq=%b ne=%b lt=%b gt=%b le=%b ge=%b\", eq_r, neq_r, lt_r, gt_r, leq_r, geq_r);
                a = 20; b = 20; #1;
                $display(\"eq=%b ne=%b lt=%b gt=%b le=%b ge=%b\", eq_r, neq_r, lt_r, gt_r, leq_r, geq_r);
                $finish;
            end
        endmodule
    ");
    // a=10, b=20: eq=0 ne=1 lt=1 gt=0 le=1 ge=0
    assert!(sim.output[0].message.contains("eq=0"));
    assert!(sim.output[0].message.contains("lt=1"));
    // a=20, b=20: eq=1 ne=0
    assert!(sim.output[1].message.contains("eq=1"));
    assert!(sim.output[1].message.contains("ne=0"));
}

#[test]
fn test_sim_for_loop_display() {
    let sim = sim_ok("
        module test;
            initial begin
                for (int i = 0; i < 4; i++) begin
                    $display(\"i=%d\", i);
                end
                $finish;
            end
        endmodule
    ");
    assert_eq!(sim.output.len(), 4);
    assert!(sim.output[0].message.contains("i=0"));
    assert!(sim.output[3].message.contains("i=3"));
}

#[test]
fn test_sim_shift_operations() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, shl, shr;
            assign shl = a << 2;
            assign shr = a >> 1;
            initial begin
                a = 8'b0000_1100; #1;
                $display(\"shl=%b shr=%b\", shl, shr);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("shl=00110000"));
    assert!(sim.output[0].message.contains("shr=00000110"));
}

#[test]
fn test_sim_alu() {
    let sim = sim_ok("
        module test;
            logic [7:0] a, b, result;
            logic [2:0] op;
            always_comb begin
                case (op)
                    3'd0: result = a + b;
                    3'd1: result = a - b;
                    3'd2: result = a & b;
                    3'd3: result = a | b;
                    3'd4: result = a ^ b;
                    default: result = 0;
                endcase
            end
            initial begin
                a = 15; b = 10;
                op = 0; #1; $display(\"ADD: %d\", result);
                op = 1; #1; $display(\"SUB: %d\", result);
                op = 2; #1; $display(\"AND: %d\", result);
                op = 3; #1; $display(\"OR:  %d\", result);
                op = 4; #1; $display(\"XOR: %d\", result);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("ADD: 25"));
    assert!(sim.output[1].message.contains("SUB: 5"));
}

#[test]
fn test_sim_display_hex() {
    let sim = sim_ok("
        module test;
            logic [15:0] val;
            initial begin
                val = 16'hDEAD;
                $display(\"hex=%h dec=%d bin=%b\", val, val, val);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("hex=dead"));
    assert!(sim.output[0].message.contains("dec=57005"));
}

#[test]
fn test_sim_time_display() {
    let sim = sim_ok("
        module test;
            initial begin
                $display(\"t=%0t\", $time);
                #10;
                $display(\"t=%0t\", $time);
                #20;
                $display(\"t=%0t\", $time);
                $finish;
            end
        endmodule
    ");
    assert_eq!(sim.output.len(), 3);
}

#[test]
fn test_sim_finish_stops() {
    let sim = sim_ok("
        module test;
            initial begin
                $display(\"before\");
                $finish;
                $display(\"after\");
            end
        endmodule
    ");
    assert_eq!(sim.output.len(), 1);
    assert!(sim.output[0].message.contains("before"));
}

// ═══════════════════════════════════════════════════════════════════
// SEQUENTIAL LOGIC TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_sim_dff_posedge() {
    let sim = sim_ok("
        module test;
            logic clk, d, q;
            always_ff @(posedge clk) q <= d;
            initial begin
                clk = 0; d = 1; q = 0;
                #5 clk = 1;  // posedge: q captures d=1
                #1;
                $display(\"q=%b\", q);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("q=1"));
}

#[test]
fn test_sim_dff_with_reset() {
    let sim = sim_ok("
        module test;
            logic clk, rst_n;
            logic [7:0] q;
            always_ff @(posedge clk or negedge rst_n) begin
                if (!rst_n) q <= 0;
                else q <= q + 1;
            end
            initial begin
                clk = 0; rst_n = 0; q = 0;
                #5 rst_n = 1;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1;
                #1;
                $display(\"q=%d\", q);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("q=3"));
}

#[test]
fn test_sim_counter_posedge() {
    let sim = sim_ok("
        module test;
            logic clk;
            logic [3:0] count;
            always_ff @(posedge clk) count <= count + 1;
            initial begin
                clk = 0; count = 0;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1;
                #1;
                $display(\"count=%d\", count);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("count=4"));
}

#[test]
fn test_sim_nba_deferred() {
    // Non-blocking assigns should be deferred: both read old values
    let sim = sim_ok("
        module test;
            logic clk;
            logic [7:0] a, b;
            always_ff @(posedge clk) begin
                a <= b;
                b <= a;
            end
            initial begin
                clk = 0; a = 8'd10; b = 8'd20;
                #5 clk = 1;
                #1;
                $display(\"a=%d b=%d\", a, b);
                $finish;
            end
        endmodule
    ");
    // Both read old values: a gets old b (20), b gets old a (10) — swap!
    assert!(sim.output[0].message.contains("a=20"));
    assert!(sim.output[0].message.contains("b=10"));
}

#[test]
fn test_sim_blocking_vs_nonblocking() {
    // Blocking: sequential in same always block
    let sim = sim_ok("
        module test;
            logic clk;
            logic [7:0] x, y;
            always_ff @(posedge clk) begin
                x <= x + 1;
                y <= x;  // y gets OLD x (non-blocking)
            end
            initial begin
                clk = 0; x = 0; y = 0;
                #5 clk = 1; #5 clk = 0;
                #5 clk = 1;
                #1;
                $display(\"x=%d y=%d\", x, y);
                $finish;
            end
        endmodule
    ");
    // After 2 posedges: x goes 0->1->2, y gets old x: 0->0->1
    assert!(sim.output[0].message.contains("x=2"));
    assert!(sim.output[0].message.contains("y=1"));
}

#[test]
fn test_sim_clock_forever() {
    let sim = sim_ok("
        module test;
            logic clk;
            logic [3:0] count;
            always_ff @(posedge clk) count <= count + 1;
            initial begin
                clk = 0; count = 0;
                forever #5 clk = ~clk;
            end
            initial begin
                #52;
                $display(\"count=%d\", count);
                $finish;
            end
        endmodule
    ");
    // Posedges at t=5,15,25,35,45 = 5 posedges
    assert!(sim.output[0].message.contains("count=5"));
}

#[test]
fn test_sim_shift_register() {
    let sim = sim_ok("
        module test;
            logic clk, din;
            logic [3:0] sr;
            always_ff @(posedge clk) sr <= {sr[2:0], din};
            initial begin
                clk = 0; din = 1; sr = 4'b0000;
                #5 clk = 1; #5 clk = 0; // sr=0001
                din = 0;
                #5 clk = 1; #5 clk = 0; // sr=0010
                #5 clk = 1; #5 clk = 0; // sr=0100
                din = 1;
                #5 clk = 1;              // sr=1001
                #1;
                $display(\"sr=%b\", sr);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("sr=1001"));
}

#[test]
fn test_sim_negedge() {
    let sim = sim_ok("
        module test;
            logic clk;
            logic [3:0] count;
            always_ff @(negedge clk) count <= count + 1;
            initial begin
                clk = 1; count = 0;
                #5 clk = 0;  // negedge
                #5 clk = 1;
                #5 clk = 0;  // negedge
                #1;
                $display(\"count=%d\", count);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("count=2"));
}

// Regression: bytecode `expr_max_width` for `RangeSelect{Constant}` returned 1
// when slice bounds were parameter expressions like `[ENTRY_NUM-1:0]`, because
// `eval_const_expr` only handled Number/Paren/Ident and bailed on Binary{Sub}.
// The width-1 then flowed into Binary{BitAnd}'s ctx_width as Resize(_, 1),
// truncating 8-bit slices to bit 0. So `|(a[N-1:0] & b[N-1:0])` evaluated as
// `a[0] & b[0]`. Bug visible on c910 axi_fifo's pop_req. Fix: const-fold
// Binary/Unary ops in eval_const_expr; expr_max_width falls back to base
// signal width when bounds aren't const-evaluable.
#[test]
fn test_sim_param_slice_bitand_reduce_or() {
    let sim = sim_ok("
        module dut(
            input  [7:0] a,
            input  [7:0] b,
            output       y
        );
            parameter ENTRY_NUM = 8;
            assign y = |(a[ENTRY_NUM-1:0] & b[ENTRY_NUM-1:0]);
        endmodule

        module test;
            reg [7:0] a, b;
            wire y;
            dut u(.a(a), .b(b), .y(y));
            initial begin
                a = 8'h01; b = 8'h01; #1; $display(\"y=%b\", y);
                a = 8'h00; b = 8'h00; #1; $display(\"y=%b\", y);
                a = 8'h02; b = 8'h02; #1; $display(\"y=%b\", y);
                a = 8'h80; b = 8'h80; #1; $display(\"y=%b\", y);
                a = 8'h02; b = 8'h04; #1; $display(\"y=%b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("y=1"), "bit 0 set: {}", sim.output[0].message);
    assert!(sim.output[1].message.contains("y=0"), "all zero: {}", sim.output[1].message);
    // Pre-fix bug: bit 1 set returned y=0 instead of 1.
    assert!(sim.output[2].message.contains("y=1"), "bit 1 set: {}", sim.output[2].message);
    // Pre-fix bug: bit 7 set returned y=0 instead of 1.
    assert!(sim.output[3].message.contains("y=1"), "bit 7 set: {}", sim.output[3].message);
    // Mismatched bits -> AND=0 -> y=0 (correctness check on the fix).
    assert!(sim.output[4].message.contains("y=0"), "non-overlap: {}", sim.output[4].message);
}

// Regression: 3-operand bit-AND with parameter-bounded slices on all three
// operands (the actual c910 axi_fifo shape:
//   `assign pop_req = |(pop_ptr[N-1:0] & counter_done[N-1:0] & entry_vld[N-1:0])`).
#[test]
fn test_sim_param_slice_3way_reduce_or() {
    let sim = sim_ok("
        module dut(
            input  [7:0] a, b, c,
            output       y
        );
            parameter ENTRY_NUM = 8;
            assign y = |(a[ENTRY_NUM-1:0] & b[ENTRY_NUM-1:0] & c[ENTRY_NUM-1:0]);
        endmodule

        module test;
            reg [7:0] a, b, c;
            wire y;
            dut u(.a(a), .b(b), .c(c), .y(y));
            initial begin
                a = 8'h02; b = 8'hFF; c = 8'h02; #1; $display(\"y=%b\", y);
                a = 8'h00; b = 8'hFF; c = 8'h00; #1; $display(\"y=%b\", y);
                $finish;
            end
        endmodule
    ");
    assert!(sim.output[0].message.contains("y=1"), "3-way bit 1: {}", sim.output[0].message);
    assert!(sim.output[1].message.contains("y=0"), "3-way zero: {}", sim.output[1].message);
}

// Regression: parameter arithmetic in slice bounds — `[N+1:N-1]`, `[N*2-1:0]`,
// `[~N:0]`. eval_const_expr now folds Add/Sub/Mul/Div/Mod/Shift/BitAnd/BitOr/
// BitXor + unary +/-/~. Each must compute the right slice width.
#[test]
fn test_sim_param_arith_slice_widths() {
    let sim = sim_ok("
        module dut(
            input  [15:0] x,
            output [3:0]  s_sub,    // x[N-1:N-4]   width 4
            output [3:0]  s_add,    // x[N+3:N]     width 4
            output [7:0]  s_mul     // x[2*N-1:N]   width 8
        );
            parameter N = 8;
            assign s_sub = x[N-1:N-4];
            assign s_add = x[N+3:N];
            assign s_mul = x[2*N-1:N];
        endmodule

        module test;
            reg  [15:0] x;
            wire [3:0]  s_sub;
            wire [3:0]  s_add;
            wire [7:0]  s_mul;
            dut u(.x(x), .s_sub(s_sub), .s_add(s_add), .s_mul(s_mul));
            initial begin
                x = 16'hABCD; #1;
                $display(\"sub=%h add=%h mul=%h\", s_sub, s_add, s_mul);
                $finish;
            end
        endmodule
    ");
    // x = 0xABCD = 1010_1011_1100_1101
    //   N=8, x[7:4]   = 0xC (sub)
    //   x[11:8]       = 0xB (add)
    //   x[15:8]       = 0xAB (mul)
    let m = &sim.output[0].message;
    assert!(m.contains("sub=c"), "{}", m);
    assert!(m.contains("add=b"), "{}", m);
    assert!(m.contains("mul=ab"), "{}", m);
}
