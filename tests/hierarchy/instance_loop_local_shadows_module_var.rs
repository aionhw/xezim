//! A loop variable declared by the loop itself (`for (integer i = 0; ...)`,
//! `foreach (a[i])`) inside an INLINED child instance whose module also
//! declares `integer i`. The inliner prefixed every use of `i` to the child's
//! module variable (`u.i`) while the declaration stayed bare, so the loop
//! tested an x-valued `u.i` and never ran: a gray-code decoder left its
//! output at x and a FIFO read pointer never advanced. The interpreted form
//! (a `$display` in the body) had a second defect of the same shape: the loop
//! variable was written by name through the process scope hint, which for a
//! foreach re-triggered the block on its own write. Reference-verified.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const SUB: &str = "
module sub(input [4:0] rptr, input reset, output reg [4:0] a, output reg [4:0] b,
           output reg [4:0] c, output reg [4:0] d);
  integer i, j;
  // compiled: for-init local shadows the module-level i
  always @* begin: g1
    if (reset) a = 0;
    else for (integer i = 0; i <= 4; i = i + 1) a[i] = ^(rptr >> i);
  end
  // interpreted: same loop with a $display in the body
  always @* begin: g2
    for (integer i = 0; i <= 4; i = i + 1) begin
      b[i] = ^(rptr >> i);
      if (rptr == 5'b00001) $display(\"  g2 i=%0d\", i);
    end
  end
  // foreach variable shadows the module-level i, compiled and interpreted
  always @* begin: g3
    foreach (rptr[i]) c[i] = ^(rptr >> i);
  end
  always @* begin: g4
    foreach (rptr[i]) begin
      d[i] = ^(rptr >> i);
      if (rptr == 5'b00001) $display(\"  g4 i=%0d\", i);
    end
  end
endmodule
";

#[test]
fn loop_locals_shadow_the_child_module_variable() {
    let msgs = messages(&format!(
        "{SUB}
module tb;
  reg [4:0] rptr = 0; reg reset = 1; wire [4:0] a, b, c, d;
  sub u(.rptr(rptr), .reset(reset), .a(a), .b(b), .c(c), .d(d));
  initial begin
    #5 reset = 0;
    #5 rptr = 5'b10110;
    #1 $display(\"R1 %b %b %b %b i=%0d\", a, b, c, d, u.i);
    rptr = 5'b00001;
    #1 $display(\"R2 %b %b %b %b i=%0d\", a, b, c, d, u.i);
    $finish;
  end
endmodule
"
    ));
    for want in ["R1 11011 11011 11011 11011 i=x", "R2 00001 00001 00001 00001 i=x"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
    // one iteration set per activation: the interpreted blocks must not
    // re-trigger themselves through the module-level `i`
    for tag in ["  g2 i=4", "  g4 i=4"] {
        assert_eq!(msgs.iter().filter(|m| *m == tag).count(), 1, "{tag} count; got {msgs:?}");
    }
}
