//! §13.5, §11.6.1: a subroutine's input actual is evaluated as if it were
//! assigned to the formal, so the formal's declared width is the context of
//! the actual's expression. `t({2{1'b1}} << 4)` into `input [31:0] v` shifts
//! at 32 bits and binds 32'h30. Module-scope functions and class methods
//! already did this; a module-scope task evaluated the actual self-determined
//! and only resized the result, so the 2-bit replication had shifted its bits
//! out and the task bound 0. The Analog Devices JESD204 TX register-map
//! testbench passes `({NUM_LINKS{1'b1}} << 4)` to such a task and failed.
//!
//! The expected values follow the LRM; Icarus Verilog 13
//! agrees except that it evaluates a default argument self-determined.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[track_caller]
fn assert_lines(got: &[String], want: &[&str]) {
    assert_eq!(got, want, "got {got:#?}");
}

/// Task inputs: a shift, a concatenation and a replication widen to the
/// formal before they shift; unary minus, `~`, `+` and `*` take the formal's
/// width too; a signed actual sign-extends into an unsigned or signed formal
/// and an unsigned one zero-extends; a default is evaluated the same way.
#[test]
fn task_input_actuals_take_the_formal_width() {
    let src = r#"
module t;
  parameter N = 2;
  logic [7:0] a8 = 8'hff, b8 = 8'h01, n8 = 8'h0f;
  logic signed [7:0] s8 = -8'sd3;
  logic c = 1'b1;
  logic [31:0] r32;
  logic [15:0] r16;
  logic signed [15:0] rs16;
  logic [8:0] r9;
  task t32; input [31:0] v; begin r32 = v; end endtask
  task automatic ta32(input logic [31:0] v); r32 = v; endtask
  task automatic tas16(input logic signed [15:0] v); rs16 = v; endtask
  task automatic tau16(input logic [15:0] v); r16 = v; endtask
  task automatic t9(input logic [8:0] v); r9 = v; endtask
  task automatic tint(input int v); r32 = v; endtask
  task automatic tdef(input logic [31:0] v = {N{1'b1}} << 4); r32 = v; endtask
  initial begin
    t32({N{1'b1}} << 4);          $display("T01 %h", r32);
    ta32({2{1'b1}} << 4);         $display("T02 %h", r32);
    ta32({4'hf, 4'h0} << 4);      $display("T03 %h", r32);
    ta32(-b8);                    $display("T04 %h", r32);
    ta32(~n8);                    $display("T05 %h", r32);
    t9(a8 + b8);                  $display("T06 %h", r9);
    tas16(s8);                    $display("T07 %h", rs16);
    tas16(8'hfd);                 $display("T08 %h", rs16);
    tas16(s8 + 8'd1);             $display("T09 %h", rs16);
    tau16(s8);                    $display("T10 %h", r16);
    ta32(c ? 2'b11 << 4 : 8'h0);  $display("T11 %h", r32);
    tint({2{1'b1}} << 4);         $display("T12 %h", r32);
    tdef();                       $display("T13 %h", r32);
    ta32({2{1'b1}} >> 1 << 4);    $display("T14 %h", r32);
    ta32(a8 << 4);                $display("T15 %h", r32);
    ta32(a8 * 8'd2);              $display("T16 %h", r32);
  end
endmodule
"#;
    assert_lines(
        &lines(src),
        &[
            "T01 00000030",
            "T02 00000030",
            "T03 00000f00",
            "T04 ffffffff",
            "T05 fffffff0",
            "T06 100",
            "T07 fffd",
            "T08 00fd",
            "T09 00fe",
            "T10 fffd",
            "T11 00000030",
            "T12 00000030",
            "T13 00000030",
            "T14 00000010",
            "T15 00000ff0",
            "T16 000001fe",
        ],
    );
}

/// Function inputs follow the same rule, in a procedural call, a
/// continuous assignment and an `always_comb` block; pinned here beside the
/// task cases.
#[test]
fn function_input_actuals_take_the_formal_width() {
    let src = r#"
module t;
  parameter N = 2;
  logic [7:0] a8 = 8'hff, b8 = 8'h01, n8 = 8'h0f;
  logic signed [7:0] s8 = -8'sd3;
  logic [31:0] r32;
  function [31:0] f32; input [31:0] v; f32 = v; endfunction
  function automatic logic [31:0] fa32(input logic [31:0] v); return v; endfunction
  function automatic logic signed [15:0] fs16(input logic signed [15:0] v); return v; endfunction
  function automatic logic [8:0] f9(input logic [8:0] v); return v; endfunction
  function automatic logic [31:0] fdef(input logic [31:0] v = {N{1'b1}} << 4); return v; endfunction
  function automatic logic [31:0] fid2(input logic [31:0] u, input logic [31:0] v); return v; endfunction
  wire [31:0] wf = fa32({N{1'b1}} << 4);
  logic [31:0] cf;
  always_comb cf = fa32({2{1'b1}} << 8);
  initial begin
    #1;
    $display("F01 %h", f32({N{1'b1}} << 4));
    $display("F02 %h", fa32({2{1'b1}} << 4));
    $display("F03 %h", fa32(-b8));
    $display("F04 %h", f9(a8 + b8));
    $display("F05 %h", fs16(s8));
    $display("F06 %h", fs16(8'hfd));
    $display("F07 %h", fdef());
    $display("F08 %h", fid2(0, ~n8));
    r32 = fa32({2{1'b1}} << 4);   $display("F09 %h", r32);
    $display("F10 %h", wf);
    $display("F11 %h", cf);
    $display("F12 %h", fa32(a8 << 4) + 1);
  end
endmodule
"#;
    assert_lines(
        &lines(src),
        &[
            "F01 00000030",
            "F02 00000030",
            "F03 ffffffff",
            "F04 100",
            "F05 fffd",
            "F06 00fd",
            "F07 00000030",
            "F08 fffffff0",
            "F09 00000030",
            "F10 00000030",
            "F11 00000300",
            "F12 00000ff1",
        ],
    );
}

/// A formal whose bounds name a parameter (`input logic [W-1:0] v`) takes
/// the width they resolve to, in the top module, in instances with other
/// values, and through a hierarchical call.
#[test]
fn parameter_sized_input_actuals_take_the_formal_width() {
    let src = r#"
module m #(parameter W = 32);
  logic [W-1:0] r;
  task automatic tp(input logic [W-1:0] v); r = v; endtask
  function automatic logic [W-1:0] fp(input logic [W-1:0] v); return v; endfunction
  initial begin
    #1 tp({2{1'b1}} << 4); $display("P%0d1 %h", W, r);
    $display("P%0d2 %h", W, fp({2{1'b1}} << 4));
  end
endmodule
module t;
  parameter W = 16;
  logic [W-1:0] r;
  task automatic tp(input logic [W-1:0] v); r = v; endtask
  task tq; input [W-1:0] v; begin r = v; end endtask
  function automatic logic [W-1:0] fq(input logic [W-1:0] v); return v; endfunction
  m #(.W(12)) u12();
  m #(.W(40)) u40();
  initial begin
    tp({2{1'b1}} << 4); $display("P01 %h", r);
    tq({2{1'b1}} << 6); $display("P02 %h", r);
    $display("P03 %h", fq({2{1'b1}} << 4));
    u12.tp({2{1'b1}} << 4); $display("P04 %h", u12.r);
  end
endmodule
"#;
    assert_lines(
        &lines(src),
        &[
            "P01 0030",
            "P02 00c0",
            "P03 0030",
            "P04 030",
            "P121 030",
            "P122 030",
            "P401 0000000030",
            "P402 0000000030",
        ],
    );
}

/// The register-map testbench's shape: a Verilog task with non-ANSI inputs
/// that stores its argument with a nonblocking assignment.
#[test]
fn non_ansi_task_input_from_a_parameter_replication() {
    let src = r#"
module t;
  parameter NUM_LINKS = 2;
  reg [31:0] mem [0:3];
  task set_val;
  input [31:0] addr;
  input [31:0] value;
  begin
    mem[addr[3:2]] <= value;
  end
  endtask
  initial begin
    set_val('h4, ({NUM_LINKS{1'b1}} << 4));
    #1 $display("V01 %h", mem[1]);
  end
endmodule
"#;
    assert_lines(&lines(src), &["V01 00000030"]);
}

/// §11.6.1 Table 11-21: the operands of `&&`, `||`, `->` and `<->` stay
/// self-determined inside an input actual, so the formal's width does not
/// reach them: `(a << 1)` of `logic [7:0] a = 8'h80` is 8'h00 and false.
#[test]
fn logical_operands_in_an_input_actual_stay_self_determined() {
    let src = r#"
module t;
  logic [7:0] a = 8'h80;
  logic [31:0] r32;
  task automatic ta32(input logic [31:0] v); r32 = v; endtask
  function automatic logic [31:0] fa32(input logic [31:0] v); return v; endfunction
  initial begin
    ta32((a << 1) || 1'b0);     $display("L01 %h", r32);
    ta32((a << 1) && 1'b1);     $display("L02 %h", r32);
    ta32(1'b1 && (a << 1));     $display("L03 %h", r32);
    ta32(!(a << 1));            $display("L04 %h", r32);
    ta32((a << 1) -> 1'b0);     $display("L05 %h", r32);
    ta32((a << 1) <-> 1'b0);    $display("L06 %h", r32);
    ta32(((a << 1) || 1'b0) << 4); $display("L07 %h", r32);
    $display("L08 %h", fa32((a << 1) || 1'b0));
  end
endmodule
"#;
    assert_lines(
        &lines(src),
        &[
            "L01 00000000",
            "L02 00000000",
            "L03 00000000",
            "L04 00000001",
            "L05 00000001",
            "L06 00000001",
            "L07 00000000",
            "L08 00000000",
        ],
    );
}
