//! §7.2 / §11.4.5 — equality of unpacked structs whose members are stored
//! one leaf signal per member, in compiled code: a member that is itself a
//! struct (`s.f0 != last.f0`), in an `always` block, in a function the
//! block calls, and in an instance (whose variables are named by its scope
//! and may have no whole-struct placeholder). The compiled comparison read
//! the placeholder, which holds no value, so it found every pair equal,
//! while the same statement in an `initial` block compared the members.
//! Expected lines follow the LRM: the structs differ in an element of a
//! dynamic-array member.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const TYPES: &str = r#"
  typedef struct { longint l; byte unsigned d[]; } arr_t;
  typedef struct { arr_t f0; } rec_t;
"#;

#[test]
fn nested_member_compare_in_always() {
    let got = lines(&format!(
        r#"
module top;
{TYPES}
  arr_t a, b;
  rec_t s, last;
  bit tick;
  always @(tick) begin
    if (a != b) $display("flat ne"); else $display("flat eq");
    if (s.f0 != last.f0) $display("nested ne"); else $display("nested eq");
    if (!(s.f0 == last.f0)) $display("nested not-eq"); else $display("nested eq2");
  end
  initial begin
    a.d = new[2]; b.d = new[2]; a.d[0] = 3;
    s.f0.d = new[2]; last.f0.d = new[2]; s.f0.d[0] = 3;
    #1 tick = 1;
  end
endmodule
"#
    ));
    assert_eq!(got, vec!["flat ne", "nested ne", "nested not-eq"]);
}

#[test]
fn nested_member_compare_in_function_called_from_always() {
    let got = lines(&format!(
        r#"
module top;
{TYPES}
  rec_t s, last;
  bit tick;
  function automatic void chk();
    if (s.f0 != last.f0) $display("fn ne"); else $display("fn eq");
  endfunction
  always @(tick) chk();
  initial begin
    s.f0.d = new[2]; last.f0.d = new[2]; s.f0.d[0] = 3;
    #1 tick = 1;
    #1 last = s;
    #1 tick = 0;
  end
endmodule
"#
    ));
    assert_eq!(got, vec!["fn ne", "fn eq"]);
}

#[test]
fn member_compare_in_instance() {
    let got = lines(&format!(
        r#"
module sub(input logic clk);
{TYPES}
  arr_t a, b;
  rec_t s, last;
  bit tick;
  always @(tick) begin
    if (a != b) $display("inst flat ne"); else $display("inst flat eq");
    if (s.f0 != last.f0) $display("inst nested ne"); else $display("inst nested eq");
  end
  initial begin
    a.d = new[2]; b.d = new[2]; a.d[0] = 3;
    s.f0.d = new[2]; last.f0.d = new[2]; s.f0.d[0] = 3;
    #1 tick = 1;
  end
endmodule
module top;
  logic c = 0;
  sub u (.clk(c));
endmodule
"#
    ));
    assert_eq!(got, vec!["inst flat ne", "inst nested ne"]);
}
