//! §9.4.2, §7.2.1: an event control on a member of a packed struct waits on
//! that member: `@(posedge a.clk)` on the member's least significant bit, and
//! `@(a.valid)` on a change of the member's bits. A packed struct is one
//! vector, and xezim watched the whole vector: an edge was judged on its least
//! significant bit, which belongs to another member, and a change of any
//! member woke the block. A clock forwarded through a struct member to a
//! module's clock port (`.clk(b.clk)`, which becomes the child's
//! `@(posedge b.clk)`) therefore fired the child's flops on the other member's
//! edges. Caliptra-SS `otp_ctrl` forwards the OTP macro clock as
//! `prim_generic_otp_inputs_o.clk_i`, and the macro missed its Init command.
//!
//! The expected values follow the LRM; Icarus Verilog 13
//! agrees where it accepts the source.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

/// Edges on a one-bit and on a two-bit member, and changes of a member.
#[test]
fn event_controls_on_packed_struct_members() {
    let src = r#"
`timescale 1ns/1ps
typedef struct packed { logic [1:0] hi; logic clk; logic valid; } bus_t;
module top;
  bus_t a;
  int n_any_valid = 0, n_any_hi = 0;
  initial begin
    a = '0;
    #1 a.valid = 1;
    #1 a.clk = 1;
    #1 a.hi = 2'b10;
    #1 a.clk = 0;
    #1 a.hi = 2'b11;
    #1 $display("n_any_valid=%0d n_any_hi=%0d", n_any_valid, n_any_hi);
    $finish;
  end
  always @(a.valid) n_any_valid++;
  always @(a.hi) n_any_hi++;
  always @(posedge a.hi) $display("%0t posedge(a.hi) a=%b", $time, a);
  always @(posedge a.clk) $display("%0t posedge(a.clk) a=%b", $time, a);
  always @(negedge a.clk) $display("%0t negedge(a.clk) a=%b", $time, a);
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "0 negedge(a.clk) a=0000",
            "2000 posedge(a.clk) a=0011",
            "4000 negedge(a.clk) a=1001",
            "5000 posedge(a.hi) a=1101",
            "n_any_valid=2 n_any_hi=3",
        ]
    );
}

/// A clock forwarded through a packed-struct member to a module's clock
/// port: the receiving flop samples the data before the edge, as through a
/// plain `logic`.
#[test]
fn clock_forwarded_through_a_struct_member_samples_pre_edge_data() {
    let src = r#"
`timescale 1ns/1ps
typedef struct packed { logic clk; logic valid; } bus_t;
module sink (input logic clk, input logic rst_n, input logic valid, output logic s);
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) s <= 1'b0; else s <= valid;
endmodule
module top;
  logic clk = 0, rst_n = 0, v;
  always #5 clk = ~clk;
  always_ff @(posedge clk or negedge rst_n) if (!rst_n) v <= 1'b0; else v <= ~v;
  bus_t a; always_comb begin a.clk = clk; a.valid = v; end
  logic sa; sink ua(.clk(a.clk), .rst_n(rst_n), .valid(a.valid), .s(sa));
  bus_t b; assign b = '{clk: clk, valid: v};
  logic sb; sink ub(.clk(b.clk), .rst_n(rst_n), .valid(b.valid), .s(sb));
  logic c_clk, c_v; always_comb begin c_clk = clk; c_v = v; end
  logic sc; sink uc(.clk(c_clk), .rst_n(rst_n), .valid(c_v), .s(sc));
  initial begin
    #12 rst_n = 1;
    repeat (4) begin
      @(negedge clk);
      $display("t=%0t A=%b B=%b C=%b", $time, sa === ~v, sb === ~v, sc === ~v);
    end
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "t=20000 A=1 B=1 C=1",
            "t=30000 A=1 B=1 C=1",
            "t=40000 A=1 B=1 C=1",
            "t=50000 A=1 B=1 C=1",
        ]
    );
}

/// The member's own edge also drives an `always` block whose `begin ... end`
/// opens with the event control, and a control that names the member through
/// the top module (`top.a.clk`). A procedural wait already did. Here `valid`,
/// the struct's least significant bit, rises at 1000 and 4000 and `clk` at
/// 2000 and 6000.
#[test]
fn wrapped_and_top_qualified_member_controls() {
    let src = r#"
`timescale 1ns/1ps
typedef struct packed { logic [1:0] hi; logic clk; logic valid; } bus_t;
module top;
  bus_t a;
  int n_w = 0, n_q = 0, n_p = 0;
  initial begin
    a = '0;
    #1 a.valid = 1;
    #1 a.clk = 1;
    #1 a.valid = 0;
    #1 a.valid = 1;
    #1 a.clk = 0;
    #1 a.clk = 1;
    #1 a.valid = 0;
    #1 $display("n_w=%0d n_q=%0d n_p=%0d", n_w, n_q, n_p);
    $finish;
  end
  always begin @(posedge a.clk); n_w++; $display("%0t wrapped a=%b", $time, a); end
  always @(posedge top.a.clk) begin n_q++; $display("%0t qualified a=%b", $time, a); end
  initial forever begin @(posedge a.clk) n_p++; $display("%0t procedural a=%b", $time, a); end
endmodule
"#;
    let mut got = lines(src);
    got.sort();
    assert_eq!(
        got,
        [
            "2000 procedural a=0011",
            "2000 qualified a=0011",
            "2000 wrapped a=0011",
            "6000 procedural a=0011",
            "6000 qualified a=0011",
            "6000 wrapped a=0011",
            "n_w=2 n_q=2 n_p=2",
        ]
    );
}
