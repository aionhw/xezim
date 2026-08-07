//! §23.8: a string formal's name is local to its frame and must not leak
//! into a later subroutine's integral formal of the same name.
//!
//! xezim uses a shared `string_signals` set to route `s[i]` to a CHARACTER
//! (byte) select instead of a bit-select.  Class-method and free-function
//! dispatch inserted every string formal / string return name into that set
//! but never removed it on return, so once any subroutine with a `string`
//! formal named `value` ran (e.g. UVM's `uvm_packer::pack_string(string
//! value)`), the name `value` stayed marked as a string forever.  A *later*
//! `pack_field_int(uvm_integral_t value)` then byte-selected `value[i]`
//! instead of bit-selecting it, silently corrupting every packed integral
//! field — the root cause of the UVM pack/unpack failures.
//!
//! The fix tracks only the names *newly* added by this frame
//! (`HashSet::insert` returns false when the name is already present from an
//! active caller, so nested recursion is handled) and removes exactly those
//! on exit.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// A class with a `string value` formal followed by an integral `value`
/// formal.  Calling the string method first must not make the integral
/// method's `value[i]` byte-select.
const CLASS_METHOD_LEAK: &str = r#"
class helper;
  static function void str_fn(string value);
    if (value.len() > 0) ;
  endfunction
  static function logic [63:0] int_fn(logic signed [63:0] value);
    logic [63:0] acc;
    acc = 0;
    for (int i = 0; i < 64; i++)
      acc[i] = value[i];
    return acc;
  endfunction
endclass
module top;
  initial begin
    logic signed [63:0] val;
    logic [63:0] result;
    helper::str_fn("hello");
    val = 64'hDEAD_BEEF_CAFE_BABE;
    result = helper::int_fn(val);
    $display("RESULT=%016x", result);
  end
endmodule
"#;

#[test]
fn class_method_string_formal_does_not_leak() {
    let sim = simulate(CLASS_METHOD_LEAK, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let line = msgs.iter().find(|m| m.starts_with("RESULT=")).unwrap_or_else(|| {
        panic!("no RESULT line; output: {:?}", msgs)
    });
    assert!(
        line.contains("deadbeefcafebabe"),
        "expected bit-select round-trip, got byte-select garbage: {}\noutput: {:?}",
        line,
        msgs
    );
}

/// The same pattern via free functions (non-class).  The free-function
/// dispatch path had the identical leak.
const FREE_FUNCTION_LEAK: &str = r#"
function void str_fn(string value);
  if (value.len() > 0) ;
endfunction
function logic [63:0] int_fn(logic signed [63:0] value);
  logic [63:0] acc;
  acc = 0;
  for (int i = 0; i < 64; i++)
    acc[i] = value[i];
  return acc;
endfunction
module top;
  initial begin
    logic signed [63:0] val;
    logic [63:0] result;
    str_fn("hello");
    val = 64'hDEAD_BEEF_CAFE_BABE;
    result = int_fn(val);
    $display("RESULT=%016x", result);
  end
endmodule
"#;

#[test]
fn free_function_string_formal_does_not_leak() {
    let sim = simulate(FREE_FUNCTION_LEAK, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let line = msgs.iter().find(|m| m.starts_with("RESULT=")).unwrap_or_else(|| {
        panic!("no RESULT line; output: {:?}", msgs)
    });
    assert!(
        line.contains("deadbeefcafebabe"),
        "expected bit-select round-trip, got byte-select garbage: {}\noutput: {:?}",
        line,
        msgs
    );
}

/// A string return variable named after the function must not leak either.
/// `get_name` returns a string; `compute` returns an integral and indexes
/// its `value` formal bit-by-bit.
const STRING_RETURN_LEAK: &str = r#"
class helper;
  static function string get_name;
    get_name = "leak-test";
  endfunction
  static function logic [31:0] compute(logic [31:0] value);
    logic [31:0] acc;
    acc = 0;
    for (int i = 0; i < 32; i++)
      acc[i] = value[i];
    return acc;
  endfunction
endclass
module top;
  initial begin
    string dummy;
    logic [31:0] val;
    logic [31:0] result;
    dummy = helper::get_name();
    val = 32'hCAFEBABE;
    result = helper::compute(val);
    $display("RESULT=%08x", result);
  end
endmodule
"#;

#[test]
fn string_return_name_does_not_leak() {
    let sim = simulate(STRING_RETURN_LEAK, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let line = msgs.iter().find(|m| m.starts_with("RESULT=")).unwrap_or_else(|| {
        panic!("no RESULT line; output: {:?}", msgs)
    });
    // get_name returns a string; its implicit return var "get_name" must not
    // leak.  But more importantly, "value" formal of compute must bit-select.
    assert!(
        line.contains("cafebabe"),
        "expected bit-select round-trip, got garbage: {}\noutput: {:?}",
        line,
        msgs
    );
}

/// Nested calls: while the string-valued function is still on the stack, the
/// name is correctly marked as string.  It must be removed once it returns.
/// This guards against over-aggressive removal that would break legitimate
/// nested string indexing.
const NESTED_STRING_ACCESS: &str = r#"
class helper;
  static function logic [7:0] first_char(string value);
    // value[0] is a character select while this frame is active
    first_char = value[0];
  endfunction
  static function logic [63:0] int_fn(logic signed [63:0] value);
    logic [63:0] acc;
    acc = 0;
    for (int i = 0; i < 64; i++)
      acc[i] = value[i];
    return acc;
  endfunction
endclass
module top;
  initial begin
    logic [63:0] r1, r2;
    // first_char uses string indexing on "value"
    r1 = helper::first_char("ABCD");   // 'A' = 0x41
    // after it returns, int_fn's value[i] must bit-select
    r2 = helper::int_fn(64'hDEAD_BEEF_CAFE_BABE);
    $display("R1=%016x R2=%016x", r1, r2);
  end
endmodule
"#;

#[test]
fn nested_string_access_then_integral() {
    let sim = simulate(NESTED_STRING_ACCESS, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let line = msgs.iter().find(|m| m.starts_with("R1=")).unwrap_or_else(|| {
        panic!("no R1 line; output: {:?}", msgs)
    });
    // r1 = first_char("ABCD") = 'A' = 0x41 (string char select worked)
    assert!(
        line.contains("R1=0000000000000041"),
        "expected string char select 'A'=0x41, got: {}\noutput: {:?}",
        line,
        msgs
    );
    // r2 = int_fn(deadbeefcafebabe) must bit-select correctly
    assert!(
        line.contains("R2=deadbeefcafebabe"),
        "expected bit-select round-trip, got garbage: {}\noutput: {:?}",
        line,
        msgs
    );
}
