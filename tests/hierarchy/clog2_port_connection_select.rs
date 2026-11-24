//! §11.5.1, §20.8.1, §23.3.3: a part-select bounded by `$clog2(...)` in a
//! port connection (`.raddr({a[$clog2(N)-1:3], 3'b000})`), read by a clocked
//! block of the child.
//!
//! The child's reads of the port compile the connection expression. The
//! compiler did not fold `$clog2` of a constant, so the bounds counted as
//! dynamic, and the dynamic part-select used the labels as physical bit
//! positions: on `input [11:2] a` the select `a[11:3]` read bits 11..3 of a
//! 10-bit vector, two of them past its end. The clocked read saw x in the
//! upper bits while a continuous assignment, evaluated by the interpreter,
//! read the right value. `$clog2` of a constant now folds, and a bound that
//! still does not fold is mapped through the declared range (an ascending
//! range is left to the interpreter). This kept the VeeRwolf SoC's boot ROM
//! from returning instructions.
//!
//! Expected values are Verilator's (5.046).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn assert_has(src: &str, expected: &[&str]) {
    let got = lines(src);
    for e in expected {
        assert!(got.iter().any(|l| l == e), "missing `{e}` in {got:#?}");
    }
}

/// The issue's testcase: a clocked and a continuous read of the port.
#[test]
fn clog2_bound_in_a_port_connection() {
    assert_has(
        r#"
module leaf (input wire clk, input wire [11:0] raddr, output reg [11:0] q, output wire [11:0] qa);
  always @(posedge clk) q <= raddr;
  assign qa = raddr;
endmodule
module wrap #(parameter N = 4096) (input wire clk, input wire [11:2] a, output wire [11:0] q, output wire [11:0] qa);
  wire [11:0] direct = {a[$clog2(N)-1:3], 3'b000};
  leaf r (.clk(clk), .raddr({a[$clog2(N)-1:3], 3'b000}), .q(q), .qa(qa));
  initial #2 $display("k7: direct=%h $bits=%0d", direct, $bits({a[$clog2(N)-1:3], 3'b000}));
endmodule
module tb;
  reg clk = 0; wire [11:0] q, qa;
  wrap w (.clk(clk), .a(10'h3ff), .q(q), .qa(qa));
  initial begin #1 clk = 1; #1 $display("k7: q=%h qa=%h", q, qa); end
endmodule
"#,
        &["k7: direct=ff8 $bits=12", "k7: q=ff8 qa=ff8"],
    );
}

/// The VeeRwolf boot ROM shape: the memory index is a part-select of the
/// connected port, `mem[raddr[AW-1:3]]`, read on the clock.
#[test]
fn boot_rom_read_through_the_port() {
    assert_has(
        r#"
module dpram64 #(parameter SIZE = 0) (input wire clk, input wire [$clog2(SIZE)-1:0] raddr, output reg [63:0] dout);
  localparam AW = $clog2(SIZE);
  reg [63:0] mem [0:SIZE/8-1];
  always @(posedge clk) dout <= mem[raddr[AW-1:3]];
endmodule
module wrap #(parameter MEM_SIZE = 0) (input wire clk, input wire [$clog2(MEM_SIZE)-1:2] i_wb_adr, output wire [63:0] q);
  dpram64 #(.SIZE(MEM_SIZE)) ram (.clk(clk), .raddr({i_wb_adr[$clog2(MEM_SIZE)-1:3],3'b000}), .dout(q));
endmodule
module tb;
  reg clk = 0; wire [63:0] q; reg [31:0] adr = 32'h4;
  wrap #(.MEM_SIZE(32'h1000)) bootrom (.clk(clk), .i_wb_adr(adr[11:2]), .q(q));
  initial begin
    bootrom.ram.mem[0] = 64'h7c029073aaaa02b7;
    #1 clk = 1; #1 $display("h2: q=%h raddr=%h", q, bootrom.ram.raddr);
  end
endmodule
"#,
        &["h2: q=7c029073aaaa02b7 raddr=000"],
    );
}

/// Bounds that do not fold at compile time (a constant function), on a
/// descending range that does not start at 0 and on an ascending one, and a
/// `$clog2` base of an indexed part-select.
#[test]
fn unfolded_bounds_map_through_the_declared_range() {
    assert_has(
        r#"
module leaf (input wire clk, input wire [11:2] a, input wire [2:11] b,
             output reg [8:0] qa, output reg [8:0] qb, output reg [3:0] qc);
  function automatic int f(int x); return x + 7; endfunction
  always @(posedge clk) begin
    qa <= a[f(4):3];
    qb <= b[3:f(4)];
    qc <= a[$clog2(4096)-1 -: 4];
  end
endmodule
module tb;
  reg clk = 0; wire [8:0] qa, qb; wire [3:0] qc;
  leaf u (.clk(clk), .a(10'b1010011101), .b(10'b1100101011), .qa(qa), .qb(qb), .qc(qc));
  initial begin #1 clk = 1; #1 $display("j1: qa=%b qb=%b qc=%b", qa, qb, qc); end
endmodule
"#,
        &["j1: qa=101001110 qb=100101011 qc=1010"],
    );
}

/// A `$clog2` argument wider than 32 bits keeps its value in the fold.
#[test]
fn wide_clog2_argument() {
    assert_has(
        r#"
// a $clog2 argument wider than 32 bits in a clocked select bound
module leaf (input wire clk, input logic [31:0] a, output logic [31:0] q);
  always @(posedge clk) q <= a[$clog2(64'h1_0000_0000)-1:0];
endmodule
module tb;
  reg clk = 0; logic [31:0] q;
  leaf u (.clk(clk), .a(32'hDEADBEEF), .q(q));
  initial begin #1 clk = 1; #1 $display("c3w: q=%h", q); end
endmodule
"#,
        &["c3w: q=deadbeef"],
    );
}

/// A folded `$clog2` is a signed 32-bit value, as the interpreter types it,
/// so `$clog2(1) - 1 < 0` is true in a clocked block. Its operand is folded
/// only when it is a literal or a parameter: an operator expression is
/// self-determined (§11.6.1), so `$clog2(8'hff + 8'h01)` is 0 and
/// `$clog2(64'd1 << 33)` is 33, and the interpreter computes both. A frame
/// local `I` hides `parameter I` (§23.9).
#[test]
fn folded_clog2_is_signed_and_only_of_a_literal_or_parameter() {
    assert_has(
        r#"
// a folded $clog2 is signed; an operator operand is self-determined; a frame local hides a parameter
module c3s;
  parameter int N = 4;
  parameter int I = 4;
  logic clk = 0;
  logic [63:0] a = 64'hfedc_ba9b_7654_3210;
  logic lt1, lt2, lt3;
  logic [63:0] q0, q1, q2;
  logic [7:0] m, mf;
  int sel = 100;
  function automatic int g(input int k);
    int I;
    I = k;
    return $clog2(I);
  endfunction
  always @(posedge clk) begin
    lt1 <= ($clog2(1) - 1 < 0);
    lt2 <= ($clog2(N) - 4 < 0);
    q0 <= a[$clog2(8'hff + 8'h01):0];
    q1 <= a[$clog2(64'd1 << 33)-1:0];
    q2 <= a[$clog2(64'h1_0000_0000 + 64'h1):0];
  end
  always @(posedge clk) begin
    int I;
    I = sel;
    m <= $clog2(I);
  end
  always_comb mf = g(sel);
  assign lt3 = ($clog2(1) - 1 < 0);
  initial begin
    #1 clk = 1;
    #1 $display("c3s: lt=%b%b%b q0=%h q1=%h q2=%h m=%0d mf=%0d", lt1, lt2, lt3, q0, q1, q2, m, mf);
  end
endmodule
"#,
        &["c3s: lt=111 q0=0000000000000000 q1=0000000176543210 q2=0000000376543210 m=7 mf=7"],
    );
}

/// §7.4.1: on a packed multidimensional base, a part-select's bounds name
/// whole elements. Bounds that do not fold (a constant function, `$clog2` of
/// an operator expression) leave the select to the interpreter, which scales
/// them; the compiled bit-range select read `a[1:0]` as two bits.
#[test]
fn unfolded_bounds_on_a_packed_array_select_elements() {
    assert_has(
        r#"
// bounds that do not fold on a packed multi-dimensional base select whole elements
module pmd;
  logic clk = 0;
  logic [3:0][7:0] a = 32'h1234567b;
  logic [31:0] q0, q1, i0, i1;
  function automatic int f(int x); return x + 1; endfunction
  always @(posedge clk) begin
    q0 <= a[$clog2(2+2)-1:0];
    q1 <= a[f(2):f(0)];
  end
  initial begin
    i0 = a[$clog2(2+2)-1:0];
    i1 = a[f(2):f(0)];
    #1 clk = 1;
    #1 $display("pmd: q0=%h q1=%h i0=%h i1=%h", q0, q1, i0, i1);
  end
endmodule
"#,
        &["pmd: q0=0000567b q1=00123456 i0=0000567b i1=00123456"],
    );
}

/// §20.8.1 (#1606): both the operand width and any high set bits survive.
#[test]
fn clog2_wide_operands_in_port_bounds_and_runtime() {
    assert_has(
        r#"
module leaf #(parameter bit [63:0] WIDTH = 128)
    (input logic [$clog2({1'b0, WIDTH}+1)-1:0] sel);
  initial $display("port width=%0d", $bits(sel));
endmodule
module tb;
  localparam logic [128:0] HIGH = 129'd1 << 117;
  logic [128:0] a;
  leaf u (8'd0);
  initial begin
    a = HIGH + 1;
    $display("logs=%0d,%0d,%0d", $clog2(HIGH), $clog2(a), $clog2(a - 1));
  end
endmodule
"#,
        &["port width=8", "logs=117,118,117"],
    );
}
