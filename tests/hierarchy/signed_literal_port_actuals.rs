//! IEEE 1800-2023 §23.3.3.8: an integer literal (§5.7.1) feeds the formal
//! through an assignment that gives the result the formal's width and sign
//! (§10.7, §11.4.4, §11.8.1–2).

use xezim::simulate;

/// An ANSI port with an implicit signed type inherits the input direction
/// (§23.2.2.3). Its own packed dimensions determine its width.
/// The same assignment rule applies to non-ANSI ports (§23.2.2.1) and
/// instances in loop generate constructs (§27.4).
#[test]
fn signed_input_reads_integer_literal_as_the_formal_type() {
    let src = r#"
`timescale 1ns/1ps
module scalar_child(input wire [7:0] a, signed b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module vector_child(input wire a, signed [3:0] b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module explicit_child(input signed b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module nonansi_child(b);
  input wire signed b;
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module word_child(input wire signed [31:0] b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module default_child(input wire signed [7:0] b = 8'hf0);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module top;
  scalar_child plain(8'hff, 1'b1);
  scalar_child paren(8'hff, ((1'b1)));
  vector_child sized(1'b0, 4'hf);
  explicit_child explicit_port(1'b1);
  nonansi_child nonansi(1'b1);
  word_child unsized('hffffffff);
  default_child by_default();
  for (genvar i = 0; i < 1; ++i) begin : g
    scalar_child generated(8'hff, 1'b1);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let messages: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    for expected in [
        "top.plain negative=1",
        "top.paren negative=1",
        "top.sized negative=1",
        "top.explicit_port negative=1",
        "top.nonansi negative=1",
        "top.unsized negative=1",
        "top.by_default negative=1",
        "top.g[0].generated negative=1",
    ] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:?}"
        );
    }
}

/// Both directions of §23.3.3.8 apply. The concatenation is unsigned
/// (§11.8.1), and assignments extend each actual according to its own sign (§10.7).
#[test]
fn unsigned_input_and_matching_or_nonliteral_actuals() {
    let src = r#"
`timescale 1ns/1ps
module signed_child(input wire signed b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module unsigned_child(input wire b);
  initial #1 $display("%m negative=%b", b < 0);
endmodule
module signed_parent(input wire signed b);
  unsigned_child nested(b);
endmodule
module signed_vector(input wire signed [3:0] b);
  initial #1 $display("%m bits=%b negative=%b", b, b < 0);
endmodule
module unsigned_vector(input wire [3:0] b);
  initial #1 $display("%m bits=%b negative=%b", b, b < 0);
endmodule
module passthrough(input wire b, output wire o, inout wire io);
  assign o = b;
  assign io = b;
endmodule
module sized_actuals(input wire en, input wire [31:0] a);
  initial #1 $display("%m en=%b a_bits=%0d a=%0d", en, $bits(a), a);
endmodule
module top;
  parameter logic P = 1'b1;
  localparam logic L = 1'b1;
  wire u = 1'b1;
  wire signed s = 1'sb1;
  wire o, io;
  unsigned_child opposite(1'sb1);
  signed_parent chain(1'sb1);
  // Guards for matching types, expression actuals and lvalue connections.
  unsigned_child matching_unsigned(1'b1);
  signed_child matching_signed(1'sb1);
  sized_actuals matching_sized(.en(1'b1), .a(32'd5));
  signed_child concat_actual({1'b1});
  signed_child unsigned_wire(u);
  unsigned_child signed_wire(s);
  signed_vector zero_extend(1'b1);
  unsigned_vector sign_extend(1'sb1);
  signed_vector fill_actual('1);
  signed_child parameter_actual(P);
  signed_child localparam_actual(L);
  passthrough ports(u, o, io);
  initial #1 $display("output=%b inout=%b", o, io);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let messages: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    for expected in [
        "top.opposite negative=0",
        "top.chain.nested negative=0",
        "top.matching_unsigned negative=0",
        "top.matching_signed negative=1",
        "top.matching_sized en=1 a_bits=32 a=5",
        "top.concat_actual negative=1",
        "top.unsigned_wire negative=1",
        "top.signed_wire negative=0",
        "top.zero_extend bits=0001 negative=0",
        "top.sign_extend bits=1111 negative=0",
        "top.fill_actual bits=1111 negative=1",
        "top.parameter_actual negative=1",
        "top.localparam_actual negative=1",
        "output=1 inout=1",
    ] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:?}"
        );
    }
}

/// Unsized literals still take the formal's width under §10.7. This applies
/// even when the literal and formal agree on signedness (§5.7.1).
#[test]
fn unsized_input_literals_take_the_formal_width() {
    let src = r#"
`timescale 1ns/1ps
module enable(input wire en, rst);
  initial #1 $display("%m en_bits=%0d inv_en=%b rst_bits=%0d inv_rst=%b",
                      $bits(en), ~en, $bits(rst), ~rst);
endmodule
module unsigned_byte(input wire [7:0] k);
  initial #1 $display("%m bits=%0d value=%0d", $bits(k), k);
endmodule
module signed_byte(input wire signed [7:0] k);
  initial #1 $display("%m bits=%0d value=%0d", $bits(k), k);
endmodule
module top;
  enable tie_off(.en(1), .rst(0));
  unsigned_byte decimal(.k(300));
  unsigned_byte based(.k('h1ff));
  signed_byte same_sign(.k(200));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    for expected in [
        "top.tie_off en_bits=1 inv_en=0 rst_bits=1 inv_rst=1",
        "top.decimal bits=8 value=44",
        "top.based bits=8 value=255",
        "top.same_sign bits=8 value=-56",
    ] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:?}"
        );
    }
}

/// An input assignment also drops X/Z for a 2-state type (§6.11.2, §6.18,
/// §6.20.3), converts reals (§6.12.1), and sizes string literals (§5.9).
/// A tri0 or tri1 input resolves high impedance to its pull value (§6.6.5).
#[test]
fn input_literals_follow_assignment_conversion() {
    let src = r#"
`timescale 1ns/1ps
typedef bit [3:0] nibble_t;
module bit_byte(input var bit signed [7:0] b);
  initial #1 $display("%m value=%0d", b);
endmodule
module aliased_bit(input var nibble_t b);
  initial #1 $display("%m bits=%b", b);
endmodule
module parameterized_bit #(parameter type T = logic)(input var T b);
  initial #1 $display("%m bits=%b", b);
endmodule
module four_state(input logic [7:0] b);
  initial #1 $display("%m bits=%b", b);
endmodule
module pulls(input tri0 [3:0] low, input tri1 high);
  initial #1 $display("%m low=%b high=%b", low, high);
endmodule
module byte_net(input wire [7:0] b);
  initial #1 $display("%m bits=%0d value=%0d", $bits(b), b);
endmodule
module actual_real(input real b);
  initial #1 $display("%m half=%0.2f", b / 2);
endmodule
module actual_string(input string b);
  initial #1 $display("%m len=%0d value=%s", b.len(), b);
endmodule
`timescale 1ps/1ps
module top;
  bit_byte drop_x(8'hfx);
  aliased_bit drop_alias_x(4'b1x01);
  parameterized_bit #(.T(bit [3:0])) drop_parameter_z(4'b1z01);
  four_state keep_x(8'bx0x1x0x1);
  pulls resolve_z(4'b1z0z, 1'bz);
  byte_net from_string("AB");
  byte_net from_real(2.5);
  // §5.8: the actual's 25 ps is 25 units of the connecting scope.
  byte_net from_time(25ps);
  actual_real to_real(5);
  // A dynamic string has no fixed packed width (§6.16).
  actual_string string_guard("AB");
endmodule
"#;
    let sim = simulate(src, 2000).expect("simulate");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    for expected in [
        "top.drop_x value=-16",
        "top.drop_alias_x bits=1001",
        "top.drop_parameter_z bits=1001",
        "top.keep_x bits=x0x1x0x1",
        "top.resolve_z low=1000 high=1",
        "top.from_string bits=8 value=66",
        "top.from_real bits=8 value=3",
        "top.from_time bits=8 value=25",
        "top.to_real half=2.50",
        "top.string_guard len=2 value=AB",
    ] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:?}"
        );
    }
}

#[test]
fn concatenation_actual_resolves_through_pulled_formals() {
    let sim = simulate(
        r#"
`timescale 1ns/1ns
module sink(input tri0 [3:0] lo, input tri1 [3:0] hi);
  initial begin
    #1 $display("first %b %b", lo, hi);
    #2 $display("second %b %b", lo, hi);
  end
endmodule
module top;
  logic [1:0] v;
  sink u({v, 2'bz0}, {v, 2'bz0});
  initial begin v = 2'b1z; #2 v = 2'bz0; end
endmodule
"#,
        10,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert!(lines.contains(&"first 1000 1110"), "{lines:?}");
    assert!(lines.contains(&"second 0000 1010"), "{lines:?}");
}
