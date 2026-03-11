//! IEEE 1800-2017/2023 Compliance Tests.
//!
//! Each test corresponds to specific sections of the LRM and verifies
//! that the parser correctly handles the constructs defined therein.
//! Test names follow: test_lrm_<section>_<feature>

use sisvsim::*;
use sisvsim::ast::*;
use sisvsim::ast::module::*;
use sisvsim::ast::decl::*;
use sisvsim::ast::types::*;
use sisvsim::ast::expr::*;
use sisvsim::ast::stmt::*;

/// Helper: parse and assert no errors
fn parse_ok(src: &str) -> ParseResult {
    let result = parse_str(src).expect("parse_str failed");
    if result.has_errors() {
        for d in &result.diagnostics {
            eprintln!("  {}", d);
        }
    }
    assert!(!result.has_errors(), "Expected no parse errors");
    result
}

/// Helper: get first module from parse result
fn first_module(result: &ParseResult) -> &crate::ast::module::ModuleDeclaration {
    match &result.source_text.descriptions[0] {
        Description::Module(m) => m,
        _ => panic!("Expected module"),
    }
}

// ═══════════════════════════════════════════════════════════════════
// §5 LEXICAL CONVENTIONS
// ═══════════════════════════════════════════════════════════════════

/// §5.6 - Identifiers, keywords, system names
#[test]
fn test_lrm_5_6_identifiers() {
    // Simple, escaped, and system identifiers
    parse_ok(r"module m;
        wire abc;
        wire \my-wire ;
        initial $display(abc);
    endmodule");
}

/// §5.7 - Numbers
#[test]
fn test_lrm_5_7_integer_literals() {
    parse_ok("module m;
        parameter int A = 659;
        parameter int B = 'h837FF;
        parameter int C = 'o7460;
        parameter int D = 4'b1001;
        parameter int E = 5'd3;
        parameter int F = 3'b01x;
        parameter int G = 12'hx;
        parameter int H = 16'hz;
    endmodule");
}

/// §5.7.1 - Real literal constants
#[test]
fn test_lrm_5_7_real_literals() {
    parse_ok("module m;
        real a = 1.2;
        real b = 0.1;
        real c = 2394.26331;
        real d = 1.2E12;
        real e = 1.30e-2;
        real f = 236.123_763_e-12;
    endmodule");
}

/// §5.9 - String literals
#[test]
fn test_lrm_5_9_string_literals() {
    parse_ok(r#"module m;
        initial begin
            $display("Hello, World!");
            $display("Line1\nLine2");
            $display("Tab\there");
        end
    endmodule"#);
}

// ═══════════════════════════════════════════════════════════════════
// §6 DATA TYPES
// ═══════════════════════════════════════════════════════════════════

/// §6.3 - Integer data types
#[test]
fn test_lrm_6_3_integer_types() {
    parse_ok("module m;
        bit a;
        logic b;
        reg c;
        byte d;
        shortint e;
        int f;
        longint g;
        integer h;
        time t;
    endmodule");
}

/// §6.3 - Signed/unsigned
#[test]
fn test_lrm_6_3_signed_unsigned() {
    parse_ok("module m;
        logic signed [7:0] s;
        logic unsigned [7:0] u;
        int unsigned ui;
        byte signed sb;
    endmodule");
}

/// §6.5 - Real/shortreal/realtime
#[test]
fn test_lrm_6_5_real_types() {
    parse_ok("module m;
        real r;
        shortreal sr;
        realtime rt;
    endmodule");
}

/// §6.6 - Void type
#[test]
fn test_lrm_6_6_void() {
    parse_ok("module m;
        function void do_nothing();
            return;
        endfunction
    endmodule");
}

/// §6.7 - String type
#[test]
fn test_lrm_6_7_string() {
    parse_ok("module m;
        string s;
    endmodule");
}

/// §6.8 - Chandle
#[test]
fn test_lrm_6_8_chandle() {
    parse_ok("module m;
        chandle h;
    endmodule");
}

/// §6.9 - Event
#[test]
fn test_lrm_6_9_event() {
    parse_ok("module m;
        event done;
    endmodule");
}

/// §6.11 - Enum
#[test]
fn test_lrm_6_11_enum() {
    parse_ok("module m;
        typedef enum logic [1:0] {RED, GREEN, BLUE} color_t;
        color_t pixel;
    endmodule");
}

/// §6.11 - Enum with explicit values
#[test]
fn test_lrm_6_11_enum_values() {
    parse_ok("module m;
        typedef enum int {IDLE = 0, RUN = 1, STOP = 2, ERROR = 3} state_t;
        state_t state;
    endmodule");
}

/// §6.12 - Struct
#[test]
fn test_lrm_6_12_struct() {
    parse_ok("module m;
        typedef struct packed {
            logic [7:0] addr;
            logic [31:0] data;
            logic valid;
        } packet_t;
        packet_t pkt;
    endmodule");
}

/// §6.12 - Union
#[test]
fn test_lrm_6_12_union() {
    parse_ok("module m;
        typedef union packed {
            logic [31:0] word;
            logic [3:0][7:0] bytes;
        } word_or_bytes_t;
    endmodule");
}

/// §6.14 - Packed arrays
#[test]
fn test_lrm_6_14_packed_arrays() {
    parse_ok("module m;
        logic [3:0][7:0] packed_arr;
    endmodule");
}

/// §6.15 - Unpacked arrays
#[test]
fn test_lrm_6_15_unpacked_arrays() {
    parse_ok("module m;
        logic [7:0] mem [0:255];
        int arr [10];
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §7 AGGREGATE DATA TYPES
// ═══════════════════════════════════════════════════════════════════

/// §7.2 - Structures
#[test]
fn test_lrm_7_2_nested_struct() {
    parse_ok("module m;
        typedef struct packed {
            logic [3:0] tag;
            struct packed {
                logic [7:0] addr;
                logic [31:0] data;
            } payload;
        } frame_t;
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §9 PROCESSES
// ═══════════════════════════════════════════════════════════════════

/// §9.2.1 - always_comb
#[test]
fn test_lrm_9_2_1_always_comb() {
    parse_ok("module m(input logic a, b, output logic y);
        always_comb y = a & b;
    endmodule");
}

/// §9.2.2 - always_latch
#[test]
fn test_lrm_9_2_2_always_latch() {
    parse_ok("module m(input logic clk, d, en, output logic q);
        always_latch if (en) q <= d;
    endmodule");
}

/// §9.2.3 - always_ff
#[test]
fn test_lrm_9_2_3_always_ff() {
    parse_ok("module m(input logic clk, rst, d, output logic q);
        always_ff @(posedge clk or posedge rst)
            if (rst) q <= 0;
            else q <= d;
    endmodule");
}

/// §9.3 - Procedural blocks
#[test]
fn test_lrm_9_3_initial_final() {
    parse_ok("module m;
        initial begin
            $display(\"init\");
        end
        final begin
            $display(\"final\");
        end
    endmodule");
}

/// §9.4 - Fork/join
#[test]
fn test_lrm_9_4_fork_join() {
    parse_ok("module m;
        initial begin
            fork
                #10 $display(\"A\");
                #20 $display(\"B\");
            join
        end
    endmodule");
}

/// §9.4 - Fork/join_any and join_none
#[test]
fn test_lrm_9_4_fork_join_any_none() {
    parse_ok("module m;
        initial begin
            fork
                #10 $display(\"A\");
            join_any
            fork
                #10 $display(\"B\");
            join_none
        end
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §10 ASSIGNMENT STATEMENTS
// ═══════════════════════════════════════════════════════════════════

/// §10.3 - Continuous assignments
#[test]
fn test_lrm_10_3_continuous_assign() {
    parse_ok("module m(input logic a, b, output logic y);
        assign y = a ^ b;
    endmodule");
}

/// §10.4 - Blocking and nonblocking
#[test]
fn test_lrm_10_4_blocking_nonblocking() {
    parse_ok("module m;
        logic a, b;
        always_comb a = b;
        always_ff @(posedge clk) a <= b;
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §11 OPERATORS AND EXPRESSIONS
// ═══════════════════════════════════════════════════════════════════

/// §11.3 - Operator precedence (complex expression)
#[test]
fn test_lrm_11_3_operator_precedence() {
    parse_ok("module m;
        logic [31:0] a, b, c, d;
        assign d = a + b * c;
        assign d = (a | b) & ~c;
        assign d = a << 2;
        assign d = a >>> 1;
        assign d = a ** 2;
    endmodule");
}

/// §11.4 - Concatenation and replication
#[test]
fn test_lrm_11_4_concatenation() {
    parse_ok("module m;
        logic [7:0] a, b;
        logic [15:0] c;
        assign c = {a, b};
        assign c = {2{a}};
    endmodule");
}

/// §11.5 - Conditional operator
#[test]
fn test_lrm_11_5_conditional() {
    parse_ok("module m;
        logic a, b, sel, y;
        assign y = sel ? a : b;
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §12 PROCEDURAL STATEMENTS
// ═══════════════════════════════════════════════════════════════════

/// §12.4 - If-else
#[test]
fn test_lrm_12_4_if_else_chain() {
    parse_ok("module m;
        logic [1:0] sel;
        logic [3:0] out;
        always_comb begin
            if (sel == 2'b00) out = 4'h1;
            else if (sel == 2'b01) out = 4'h2;
            else if (sel == 2'b10) out = 4'h4;
            else out = 4'h8;
        end
    endmodule");
}

/// §12.5 - Case
#[test]
fn test_lrm_12_5_case() {
    parse_ok("module m;
        logic [1:0] sel;
        logic [3:0] out;
        always_comb begin
            case (sel)
                2'b00: out = 4'h1;
                2'b01: out = 4'h2;
                2'b10: out = 4'h4;
                default: out = 4'h8;
            endcase
        end
    endmodule");
}

/// §12.5 - Unique/priority case
#[test]
fn test_lrm_12_5_unique_priority() {
    parse_ok("module m;
        logic [1:0] sel;
        logic out;
        always_comb begin
            unique case (sel)
                2'b00: out = 0;
                2'b01: out = 1;
                default: out = 0;
            endcase
        end
    endmodule");
}

/// §12.7 - Loop statements
#[test]
fn test_lrm_12_7_loops() {
    parse_ok("module m;
        initial begin
            int i;
            for (i = 0; i < 10; i++) begin
                $display(i);
            end
            while (i > 0) i = i - 1;
            repeat (5) $display(\"rep\");
            forever begin
                #10 ;
                break;
            end
        end
    endmodule");
}

/// §12.7.3 - Foreach
#[test]
fn test_lrm_12_7_3_foreach() {
    parse_ok("module m;
        int arr [10];
        initial begin
            foreach (arr[i]) arr[i] = i;
        end
    endmodule");
}

/// §12.8 - Return, break, continue
#[test]
fn test_lrm_12_8_jump_stmts() {
    parse_ok("module m;
        function int calc(input int x);
            if (x < 0) return 0;
            return x * 2;
        endfunction
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §13 TASKS AND FUNCTIONS
// ═══════════════════════════════════════════════════════════════════

/// §13.3 - Tasks
#[test]
fn test_lrm_13_3_task_with_ports() {
    parse_ok("module m;
        task automatic my_task(input int a, output int b);
            b = a + 1;
        endtask
    endmodule");
}

/// §13.4 - Functions with multiple ports
#[test]
fn test_lrm_13_4_function_multiple_ports() {
    parse_ok("module m;
        function automatic int max(input int a, input int b);
            if (a > b) return a;
            else return b;
        endfunction
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §23 MODULES AND HIERARCHY
// ═══════════════════════════════════════════════════════════════════

/// §23.2 - Module header (ANSI style)
#[test]
fn test_lrm_23_2_ansi_ports() {
    parse_ok("module adder #(parameter int WIDTH = 32) (
        input logic [WIDTH-1:0] a,
        input logic [WIDTH-1:0] b,
        output logic [WIDTH:0] sum
    );
        assign sum = a + b;
    endmodule");
}

/// §23.3 - Module instantiation
#[test]
fn test_lrm_23_3_instantiation() {
    parse_ok("module top;
        logic [7:0] x, y;
        logic [8:0] z;
        adder #(.WIDTH(8)) u_add (.a(x), .b(y), .sum(z));
    endmodule");
}

/// §23.3 - Multiple instances
#[test]
fn test_lrm_23_3_multiple_instances() {
    parse_ok("module top;
        logic a, b, y1, y2;
        my_and g1(.a(a), .b(b), .y(y1)), g2(.a(b), .b(a), .y(y2));
    endmodule");
}

/// §23.3 - Wildcard port connections
#[test]
fn test_lrm_23_3_wildcard_ports() {
    parse_ok("module top;
        logic a, b, y;
        my_mod u1(.*);
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §24 PROGRAMS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_lrm_24_program() {
    parse_ok("program test(input logic clk);
        initial begin
            $display(\"Test start\");
            #100;
            $display(\"Test end\");
            $finish;
        end
    endprogram");
}

// ═══════════════════════════════════════════════════════════════════
// §25 INTERFACES
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_lrm_25_interface() {
    parse_ok("interface simple_bus;
        logic [7:0] addr;
        logic [7:0] data;
        logic valid;
        logic ready;
    endinterface");
}

// ═══════════════════════════════════════════════════════════════════
// §26 PACKAGES
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_lrm_26_package() {
    parse_ok("package my_pkg;
        typedef logic [7:0] byte_t;
        typedef logic [15:0] halfword_t;
        parameter int MAX_SIZE = 256;
        function automatic int min(int a, int b);
            return (a < b) ? a : b;
        endfunction
    endpackage");
}

#[test]
fn test_lrm_26_import() {
    parse_ok("
    package pkg;
        typedef int my_int;
    endpackage
    module m;
        import pkg::*;
    endmodule");
}

/// §26.3 - Selective import
#[test]
fn test_lrm_26_3_selective_import() {
    parse_ok("
    package pkg;
        typedef int my_int;
    endpackage
    module m;
        import pkg::my_int;
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// §27 GENERATE CONSTRUCTS
// ═══════════════════════════════════════════════════════════════════

/// §27.3 - Generate region
#[test]
fn test_lrm_27_3_generate() {
    parse_ok("module m;
        genvar i;
        generate
            wire gen_wire;
        endgenerate
    endmodule");
}

// ═══════════════════════════════════════════════════════════════════
// COMPLEX INTEGRATION TESTS
// ═══════════════════════════════════════════════════════════════════

/// Full ALU design
#[test]
fn test_compliance_alu() {
    parse_ok("module alu #(parameter int WIDTH = 32) (
        input  logic [WIDTH-1:0] a,
        input  logic [WIDTH-1:0] b,
        input  logic [2:0]       op,
        output logic [WIDTH-1:0] result,
        output logic             zero
    );
        always_comb begin
            case (op)
                3'b000: result = a + b;
                3'b001: result = a - b;
                3'b010: result = a & b;
                3'b011: result = a | b;
                3'b100: result = a ^ b;
                3'b101: result = a << b[4:0];
                3'b110: result = a >> b[4:0];
                default: result = 0;
            endcase
            zero = (result == 0);
        end
    endmodule");
}

/// FIFO design
#[test]
fn test_compliance_fifo() {
    parse_ok("module fifo #(parameter int DEPTH = 16, parameter int WIDTH = 8) (
        input  logic             clk,
        input  logic             rst,
        input  logic             wr_en,
        input  logic             rd_en,
        input  logic [WIDTH-1:0] din,
        output logic [WIDTH-1:0] dout,
        output logic             full,
        output logic             empty
    );
        logic [WIDTH-1:0] mem [0:DEPTH-1];
        logic [3:0] wr_ptr, rd_ptr, count;

        assign full  = (count == DEPTH);
        assign empty = (count == 0);

        always_ff @(posedge clk or posedge rst) begin
            if (rst) begin
                wr_ptr <= 0;
                rd_ptr <= 0;
                count  <= 0;
            end else begin
                if (wr_en) begin
                    mem[wr_ptr] <= din;
                    wr_ptr <= wr_ptr + 1;
                end
                if (rd_en) begin
                    dout <= mem[rd_ptr];
                    rd_ptr <= rd_ptr + 1;
                end
            end
        end
    endmodule");
}

/// FSM design
#[test]
fn test_compliance_fsm() {
    parse_ok("module fsm (
        input  logic clk, rst, go, done,
        output logic start, busy
    );
        typedef enum logic [1:0] {
            IDLE  = 2'b00,
            RUN   = 2'b01,
            WAIT  = 2'b10,
            DONE  = 2'b11
        } state_t;

        state_t state, next_state;

        always_ff @(posedge clk or posedge rst)
            if (rst) state <= IDLE;
            else state <= next_state;

        always_comb begin
            next_state = state;
            start = 0;
            busy = 0;
            case (state)
                IDLE: begin
                    if (go) begin
                        next_state = RUN;
                        start = 1;
                    end
                end
                RUN: begin
                    busy = 1;
                    if (done) next_state = DONE;
                end
                DONE: next_state = IDLE;
                default: next_state = IDLE;
            endcase
        end
    endmodule");
}
