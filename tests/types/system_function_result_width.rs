//! IEEE 1800-2017 §20/§21: a system FUNCTION's result has the type the LRM
//! gives it (`$countones`, `$clog2`, `$bits`, `$size` … return `int`), so in
//! `narrow <= $countones(x) >> 3` the shift operand is at least 32 bits wide.
//! The bytecode width inference had no arm for system calls and fell to 0, so
//! a compiled `always_ff` sized the shift at the 4-bit target: a count of 24
//! became 8 and 16 became 0 before the shift. The procedural interpreter got
//! it right, so the bug only showed inside compiled blocks.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn countones_shift_keeps_int_width_in_clocked_block() {
    let msgs = messages(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [31:0] be;
  logic [3:0] step;
  logic [5:0] acc = 0;
  always_ff @(posedge clk) begin
    step <= $countones(be) >> 3;
    acc <= acc + ($countones(be) >> 3);
  end
  initial begin
    be = 32'h000000FF; @(posedge clk); #1 $display("S8=%0d A=%0d", step, acc);
    be = 32'hFFFFFF00; @(posedge clk); #1 $display("S24=%0d A=%0d", step, acc);
    be = 32'h0000FFFF; @(posedge clk); #1 $display("S16=%0d A=%0d", step, acc);
    be = 32'hFFFFFFFF; @(posedge clk); #1 $display("S32=%0d A=%0d", step, acc);
    $finish;
  end
endmodule
"#,
    );
    for want in ["S8=1 A=1", "S24=3 A=4", "S16=2 A=6", "S32=4 A=10"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}

#[test]
fn int_valued_system_functions_widen_narrow_contexts() {
    // Same shape in a plain procedural block and with the other int-valued
    // functions; each result only fits once the operand is 32 bits wide.
    let msgs = messages(
        r#"
module top;
  logic [63:0] wide = 64'hFFFF_FFFF_FFFF_FFFF;
  logic [3:0] a, b, c, d, e;
  logic [7:0] arr [0:255];
  initial begin
    a = $countones(wide) >> 4;      // 64 >> 4 = 4
    b = $clog2(1024) >> 2;          // 10 >> 2 = 2
    c = $bits(wide) >> 4;           // 64 >> 4 = 4
    d = $size(arr) >> 5;            // 256 >> 5 = 8
    e = $countbits(wide, 1'b1) >> 4; // 64 >> 4 = 4
    $display("P a=%0d b=%0d c=%0d d=%0d e=%0d", a, b, c, d, e);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        msgs.iter().any(|m| m == "P a=4 b=2 c=4 d=8 e=4"),
        "int-valued system functions must not be truncated to the target width; got {msgs:?}"
    );
}

#[test]
fn onehot_and_signed_keep_their_own_widths() {
    let msgs = messages(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] v = 8'h80;
  logic [3:0] o;
  logic [15:0] s;
  always_ff @(posedge clk) begin
    o <= $onehot(v) + 4'd2;        // 1 + 2
    s <= $signed(v) >>> 4;         // 8-bit signed 0x80 >>> 4 = 8'hF8 sign-extended in a 16-bit context
  end
  initial begin
    @(posedge clk); #1 $display("O=%0d S=%h", o, s);
    $finish;
  end
endmodule
"#,
    );
    assert!(msgs.iter().any(|m| m == "O=3 S=fff8"), "got {msgs:?}");
}

// §20.9/§21.3: `int`-valued system functions are SIGNED. A count compared
// after subtraction, and `$fgetc`'s -1 at end of file, are signed compares.
#[test]
fn int_valued_system_functions_are_signed() {
    let msgs = messages(
        r#"
module top;
  logic [7:0] v = 8'h03;
  logic clk = 0;
  always #5 clk = ~clk;
  logic neg_ff;
  always_ff @(posedge clk) neg_ff <= ($countones(v) - 8) < 0;
  initial begin
    if (($countones(v) - 8) < 0) $display("PROC neg");
    else $display("PROC pos");
    if ($clog2(2) - 5 < 0) $display("CLOG neg");
    @(posedge clk); #1 $display("FF neg=%0d", neg_ff);
    $finish;
  end
endmodule
"#,
    );
    for want in ["PROC neg", "CLOG neg", "FF neg=1"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}

// Sizing a system call by evaluating it ran the call twice: `$fgetc(fd) & 8'hff`
// consumed two bytes per statement and `$urandom % n` advanced the generator
// twice. The width now comes from the LRM result type, so each call runs once.
#[test]
fn width_inference_does_not_reevaluate_system_calls() {
    let dir = std::env::temp_dir().join("xezim_sysfn_fgetc");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bytes.bin");
    std::fs::write(&path, b"ABCDEF").unwrap();
    let src = format!(
        r#"
module top;
  int fd, a, b, c, eof;
  initial begin
    fd = $fopen("{}", "r");
    a = $fgetc(fd) & 8'hff;
    b = $fgetc(fd) & 8'hff;
    c = $fgetc(fd) + 0;
    $display("BYTES %0d %0d %0d", a, b, c);
    repeat (3) void'($fgetc(fd));
    eof = $fgetc(fd);
    if (eof < 0) $display("EOF negative");
    $fclose(fd);
    $finish;
  end
endmodule
"#,
        path.display()
    );
    let msgs = messages(&src);
    assert!(msgs.iter().any(|m| m == "BYTES 65 66 67"), "each $fgetc must run once; got {msgs:?}");
    assert!(msgs.iter().any(|m| m == "EOF negative"), "$fgetc EOF must compare below zero; got {msgs:?}");
}

// §16.9.3: `$past` with no history yet yields the operand type's default
// value at the OPERAND's width, and `$sampled(e)` is e's current value.
#[test]
fn past_default_has_operand_width_and_sampled_reads_current() {
    let msgs = messages(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] v = 8'hA5;
  int n = 0;
  always @(posedge clk) begin
    n++;
    if (n == 1) $display("PAST1=%b", $past(v));
    if (n == 2) begin
      $display("PAST2=%h SAMPLED=%h", $past(v), $sampled(v));
      $finish;
    end
  end
endmodule
"#,
    );
    assert!(msgs.iter().any(|m| m == "PAST1=xxxxxxxx"), "got {msgs:?}");
    assert!(msgs.iter().any(|m| m == "PAST2=a5 SAMPLED=a5"), "got {msgs:?}");
}
