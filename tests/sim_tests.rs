//! Simulation tests for the SystemVerilog compiler/simulator.

use sisvsim::simulate;
use sisvsim::compiler::Value;

fn sim_ok(src: &str) -> sisvsim::compiler::Simulator {
    match simulate(src, 100_000) {
        Ok(sim) => sim,
        Err(e) => panic!("Simulation failed: {}", e),
    }
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
