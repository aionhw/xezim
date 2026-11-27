use xezim::simulate;

fn check_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10_000).expect("simulate failed");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.strip_prefix("CHECK "))
        .collect();
    assert_eq!(actual, expected);
}

/// IEEE 1800-2023 §6.12, §11.4.3, §20.5: unary minus flips the sign
/// of real zero, which the bit-conversion system functions expose.
#[test]
fn real_zero_sign() {
    let src = r#"`timescale 1ns/1ps
module top;
  localparam real K = -0.0;
  real x, r, literal_zero, runtime_zero, twice, nonzero;
  shortreal sr, short_literal, short_runtime;
  realtime rt, time_literal, time_runtime;
  assign r = -x;

  initial begin
    literal_zero = -0.0;
    nonzero = -1.25;
    short_literal = -0.0;
    time_literal = -0.0;
    $display("CHECK literal %016h %016h %016h", $realtobits(literal_zero),
             $realtobits(K), $realtobits(nonzero));
    $display("CHECK parens %016h %016h", $realtobits(-(+0.0)), $realtobits(-(-0.0)));
    $display("CHECK narrow/time literal %08h %08h %016h",
             $shortrealtobits(short_literal), $shortrealtobits(-0.0), $realtobits(time_literal));

    x = 0.0;
    sr = 0.0;
    rt = 0.0;
    #1;
    runtime_zero = -x;
    twice = -(-x);
    short_runtime = -sr;
    time_runtime = -rt;
    $display("CHECK positive input %016h %016h %016h %016h",
             $realtobits(runtime_zero), $realtobits(-x), $realtobits(twice), $realtobits(r));
    $display("CHECK narrow/time positive input %08h %016h",
             $shortrealtobits(short_runtime), $realtobits(time_runtime));

    x = $bitstoreal(64'h8000000000000000);
    sr = $bitstoshortreal(32'h80000000);
    rt = x;
    #1;
    runtime_zero = -x;
    twice = -(-x);
    short_runtime = -sr;
    time_runtime = -rt;
    $display("CHECK negative input %016h %016h %016h %016h",
             $realtobits(runtime_zero), $realtobits(-x), $realtobits(twice), $realtobits(r));
    $display("CHECK narrow/time negative input %08h %016h",
             $shortrealtobits(short_runtime), $realtobits(time_runtime));
    $finish;
  end
endmodule
"#;
    check_output(
        src,
        &[
            "literal 8000000000000000 8000000000000000 bff4000000000000",
            "parens 8000000000000000 0000000000000000",
            "narrow/time literal 80000000 80000000 8000000000000000",
            "positive input 8000000000000000 8000000000000000 0000000000000000 8000000000000000",
            "narrow/time positive input 80000000 8000000000000000",
            "negative input 0000000000000000 0000000000000000 8000000000000000 0000000000000000",
            "narrow/time negative input 00000000 0000000000000000",
        ],
    );
}

/// IEEE 1800-2023 §11.4.3, §11.8.1: integral minus keeps its operand's
/// width and signedness and propagates unknown bits.
#[test]
fn integral_controls() {
    let src = r#"`timescale 1ns/1ps
module top;
  logic signed [7:0] s8;
  logic signed [31:0] s32;
  logic signed [64:0] s65;
  logic signed [128:0] s129;
  logic [7:0] u8, neg8, fill_one, fill_x;
  logic [31:0] u32, neg32;
  logic [64:0] u65, neg65;
  logic [128:0] u129, neg129;

  initial begin
    s8 = 5;
    s32 = 5;
    s65 = 5;
    s129 = 5;
    u8 = 8'h81;
    u32 = 32'h80000001;
    u65 = 65'h10000000000000001;
    u129 = 129'h100000000000000000000000000000001;
    neg8 = -u8;
    neg32 = -u32;
    neg65 = -u65;
    neg129 = -u129;
    fill_one = -'1;
    fill_x = -'x;
    $display("CHECK signed %0d %0d %0d %0d", -s8, -s32, -s65, -s129);
    $display("CHECK unsigned %02h %08h %017h %033h", neg8, neg32, neg65, neg129);
    $display("CHECK fill %02h %02h, other %0d %0d", fill_one, fill_x, -0, -(-5));
    $finish;
  end
endmodule
"#;
    check_output(
        src,
        &[
            "signed -5 -5 -5 -5",
            "unsigned 7f 7fffffff 0ffffffffffffffff 0ffffffffffffffffffffffffffffffff",
            "fill 01 xx, other 0 5",
        ],
    );
}
