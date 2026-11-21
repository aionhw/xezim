//! IEEE 1800-2023 §6.12: shortreal has binary32 precision. The nearest
//! binary32 value to 16777216.25 is 16777216.0. Check the stored numeric
//! value as well as its converted bits (§20.5).

use xezim::simulate;

fn check_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10_000).expect("simulate failed");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.trim().strip_prefix("CHECK "))
        .collect();
    assert_eq!(actual, expected);
}

const BINARY32: &str = "16777216.00 1 1";

// §10.4.1: a blocking assignment converts to the destination type.
#[test]
fn blocking_assignment() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  shortreal stored;
  initial begin
    stored = drive;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.8: a declaration initializer assigns to the declared variable type.
#[test]
fn declaration_initializer() {
    let src = r#"`timescale 1ns/1ps
module top;
  shortreal stored = 16777216.25;
  initial begin
    #1;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.18: chained typedefs preserve the underlying shortreal type.
#[test]
fn typedef_variable() {
    let src = r#"`timescale 1ns/1ps
module top;
  typedef shortreal float_t;
  typedef float_t sample_t;
  real drive = 16777216.25;
  sample_t stored;
  initial begin
    stored = drive;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.5 and §10.3.2: a single continuous assignment may drive a variable.
#[test]
fn continuous_assignment_to_variable() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  shortreal stored;
  assign stored = drive;
  initial begin
    #1;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.6.7: the net uses the data type named in the nettype declaration.
#[test]
fn scalar_nettype_copy() {
    let src = r#"`timescale 1ns/1ps
module top;
  nettype shortreal voltage32;
  voltage32 stored;
  real drive = 16777216.25;
  assign stored = drive;
  initial begin
    #1;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.8 and §13.4.1: the local and the function's result can have distinct types.
#[test]
fn function_local() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  real result;

  function automatic real relay(real source);
    shortreal stored;
    stored = source;
    return stored;
  endfunction

  initial begin
    result = relay(drive);
    $display("CHECK %.2f %0d %0d", result, result == 16777216.0,
             $shortrealtobits(result) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §13.4.1: the return expression converts to the declared result type.
#[test]
fn function_result() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  real result;

  function automatic shortreal narrow(real source);
    return source;
  endfunction

  initial begin
    result = narrow(drive);
    $display("CHECK %.2f %0d %0d", result, result == 16777216.0,
             $shortrealtobits(result) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §23.3.3: each variable port connection uses its destination type.
#[test]
fn port_relay() {
    let src = r#"`timescale 1ns/1ps
module relay(input var shortreal narrow_in, input var real wide_in,
             output var real wide_out, output var shortreal narrow_out);
  assign wide_out = narrow_in;
  assign narrow_out = wide_in;
endmodule

module top;
  real drive = 16777216.25;
  real from_input, from_output;
  relay dut(drive, drive, from_input, from_output);
  initial begin
    #1;
    $display("CHECK %.2f %0d %0d", from_input, from_input == 16777216.0,
             $shortrealtobits(from_input) == $shortrealtobits(16777216.0));
    $display("CHECK %.2f %0d %0d", from_output, from_output == 16777216.0,
             $shortrealtobits(from_output) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32, BINARY32]);
}

// §6.24.1: the cast converts as an assignment to shortreal would.
#[test]
fn explicit_cast() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  real result;
  initial begin
    result = shortreal'(drive);
    $display("CHECK %.2f %0d %0d", result, result == 16777216.0,
             $shortrealtobits(result) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §10.4.2: sample after the NBA has updated its shortreal destination.
#[test]
fn nonblocking_assignment() {
    let src = r#"`timescale 1ns/1ps
module top;
  bit clk;
  real drive = 16777216.25;
  shortreal stored;
  always @(posedge clk)
    stored <= drive;
  initial begin
    #1 clk = 1;
    #1;
    $display("CHECK %.2f %0d %0d", stored, stored == 16777216.0,
             $shortrealtobits(stored) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §7.2 and §10.3.2: continuous copies to an unpacked struct member
// use the member's type, including when the RHS is a bare identifier.
#[test]
fn continuous_copy_to_struct_member() {
    let src = r#"`timescale 1ns/1ps
module top;
  struct { shortreal narrow; } frame;
  real drive = 16777216.25;
  assign frame.narrow = drive;
  initial begin
    #1;
    $display("CHECK %.2f %0d %0d", frame.narrow, frame.narrow == 16777216.0,
             $shortrealtobits(frame.narrow) == $shortrealtobits(16777216.0));
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32]);
}

// §6.12: real and realtime both retain the binary64 value.
#[test]
fn real_and_realtime_variables() {
    let src = r#"`timescale 1ns/1ps
module top;
  real drive = 16777216.25;
  real stored;
  realtime stamp;
  initial begin
    stored = drive;
    stamp = drive;
    $display("CHECK %.2f %.2f %0d", stored, stamp,
             stored == 16777216.25 && stamp == 16777216.25);
    $finish;
  end
endmodule
"#;
    check_output(src, &["16777216.25 16777216.25 1"]);
}

// §6.6.7 and §23.3.3: a real net and real variable ports retain binary64.
#[test]
fn real_nettype_and_port_relay() {
    let src = r#"`timescale 1ns/1ps
module relay(input var real source, output var real result);
  assign result = source;
endmodule

module top;
  nettype real voltage64;
  voltage64 copied;
  real drive = 16777216.25;
  real relayed;
  assign copied = drive;
  relay dut(drive, relayed);
  initial begin
    #1;
    $display("CHECK %.2f %.2f %0d", copied, relayed,
             copied == 16777216.25 && relayed == 16777216.25);
    $finish;
  end
endmodule
"#;
    check_output(src, &["16777216.25 16777216.25 1"]);
}

// §6.18 and §7.2: each struct leaf retains its type through a typedef chain.
#[test]
fn struct_member_precision() {
    let src = r#"`timescale 1ns/1ps
module top;
  typedef shortreal float_t;
  typedef float_t sample_t;
  struct { shortreal narrow; sample_t alias_leaf; real wide; } frame;
  real drive = 16777216.25;
  initial begin
    frame.narrow = drive;
    frame.alias_leaf = drive;
    frame.wide = drive;
    $display("CHECK %.2f %0d %0d", frame.narrow, frame.narrow == 16777216.0,
             $shortrealtobits(frame.narrow) == $shortrealtobits(16777216.0));
    $display("CHECK %.2f %0d %0d", frame.alias_leaf, frame.alias_leaf == 16777216.0,
             $shortrealtobits(frame.alias_leaf) == $shortrealtobits(16777216.0));
    $display("CHECK %.2f %0d", frame.wide, frame.wide == 16777216.25);
    $finish;
  end
endmodule
"#;
    check_output(src, &[BINARY32, BINARY32, "16777216.25 1"]);
}
