//! §13.4.1: a function that builds its result by writing members of its own
//! RETURN VARIABLE (`mk.a = …; mk.b = …;`) is inlined like any other.
//!
//! Two things blocked it, and both had to go:
//!
//! 1. The purity walker rejected `mk.a` — a dotted name is normally a reach
//!    outside the function. But the head here is the function's own return
//!    variable (or a formal), which IS bound, so it stays function-local. The
//!    spelling arrives three ways: a MemberAccess node (inside a body it is
//!    not collapsed), a two-segment Ident, and a single segment containing a
//!    dot.
//! 2. Even once pure, the body had no store path: an inlined return variable
//!    lives in a REGISTER, so neither the signal splice nor the array path
//!    applies, and nothing registers a member layout for it (it is not a
//!    signal). The layout is now recorded per function at compile start and
//!    the write mask-splices the member's static bit range into the register.
//!
//! Worth 8x on this shape: 3.00s -> 0.37s over 200k calls, against 0.27s for
//! the same function written `return '{…}` (which already inlined).
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn return_variable_member_writes_match_the_reference() {
    let o = out(r#"
typedef struct packed { logic [15:0] a; logic [15:0] b; } p_t;
typedef struct packed { logic [39:0] w; logic [7:0] t; logic v; } big_t;
typedef struct packed { logic signed [15:0] s; logic [7:0] u; } sg_t;
module tb;
  // partial write: `b` is never assigned and must keep its x default
  function automatic p_t part(input int unsigned c); part.a = c[15:0]; endfunction
  // wider than 32 bits
  function automatic big_t wide(input int unsigned c);
    wide.w = {c[7:0], 32'hDEAD_BEEF}; wide.t = c[7:0]; wide.v = 1'b1;
  endfunction
  function automatic sg_t sg(input int unsigned c);
    sg.s = -16'sd1234; sg.u = c[7:0];
  endfunction
  // x into one member must not disturb the other
  function automatic p_t xz(input int unsigned c);
    xz.a = 16'hxx00; xz.b = c[15:0];
  endfunction
  // reading back a member it just wrote
  function automatic p_t rb(input int unsigned c);
    rb.a = c[15:0]; rb.b = rb.a + 16'd1;
  endfunction
  // member of a FORMAL, read side
  function automatic logic [15:0] frm(input p_t q); frm = q.a ^ q.b; endfunction
  // genuinely impure: reads module state, must stay correct
  logic [15:0] gstate;
  function automatic logic [15:0] imp(input int unsigned c);
    imp = c[15:0] + gstate;
  endfunction

  p_t r1, r4, r5; big_t r2; sg_t r3; logic [15:0] r6, r7;
  initial begin
    gstate = 16'h00FF;
    r1 = part(32'h1234_5678);
    r2 = wide(32'h0000_00AB);
    r3 = sg(32'h0000_0042);
    r4 = xz(32'h0000_9999);
    r5 = rb(32'h0000_0010);
    r6 = frm('{a:16'hAAAA, b:16'h5555});
    r7 = imp(32'h0000_0001);
    $display("P=%04x/%04x", r1.a, r1.b);
    $display("W=%010x/%02x/%b", r2.w, r2.t, r2.v);
    $display("S=%0d/%02x", r3.s, r3.u);
    $display("X=%04x/%04x", r4.a, r4.b);
    $display("B=%04x/%04x", r5.a, r5.b);
    $display("F=%04x", r6);
    $display("I=%04x", r7);
  end
endmodule
"#);
    // Reference simulator, verbatim.
    for expect in [
        "P=5678/xxxx",   // b untouched -> x
        "W=abdeadbeef/ab/1",
        "S=-1234/42",
        "X=xx00/9999",   // x in one member only
        "B=0010/0011",   // read-back of the member just written
        "F=ffff",
        "I=0100",        // impure helper still correct
    ] {
        assert!(o.contains(expect), "expected {expect} in:\n{o}");
    }
}
