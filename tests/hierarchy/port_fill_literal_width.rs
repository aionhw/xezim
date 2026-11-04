//! IEEE 1800-2023 §5.7.1 / §23.3.3: an input port connected to a fill
//! literal (`'0`, `'1`, `'x`, `'z`) has the FORMAL's width inside the
//! module. The literal was substituted for the port, so `$bits(a)` read 1
//! and `%b` printed one digit while selects still worked (#540, reported
//! by chenbhil).
//!
//! The mirror case of a part-select actual: a `[N:0]` formal bound to a
//! whole net declared with another range (`logic [0:7] a`) keeps its own
//! labels too — `p[0]` is the formal's LSB, paired with `a[7]` (§23.3.3).

use xezim::simulate;

#[test]
fn fill_literal_actual_takes_the_formal_width() {
    let src = r#"
module child (input logic [7:0] a);
  logic [7:0] c;
  always_comb c = a;
  initial #1 $display("%m $bits(a)=%0d a=%b a[7]=%b a[3:0]=%b c=%b",
                      $bits(a), a, a[7], a[3:0], c);
endmodule
module tb;
  child u_zero (.a('0));
  child u_ones (.a('1));
  child u_x    (.a('x));
  child u_sized(.a(8'hff));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"tb.u_zero $bits(a)=8 a=00000000 a[7]=0 a[3:0]=0000 c=00000000"), "{msgs:?}");
    assert!(msgs.contains(&"tb.u_ones $bits(a)=8 a=11111111 a[7]=1 a[3:0]=1111 c=11111111"), "{msgs:?}");
    assert!(msgs.contains(&"tb.u_x $bits(a)=8 a=xxxxxxxx a[7]=x a[3:0]=xxxx c=xxxxxxxx"), "{msgs:?}");
    assert!(msgs.contains(&"tb.u_sized $bits(a)=8 a=11111111 a[7]=1 a[3:0]=1111 c=11111111"), "{msgs:?}");
}

#[test]
fn ascending_whole_net_actual_keeps_the_formal_labels() {
    let src = r#"
module c(input [7:0] p, output o, output [3:0] lo);
  assign o = p[0];
  assign lo = p[3:0];
endmodule
module top;
  logic [0:7] a = 8'b1000_0001;
  wire o; wire [3:0] lo;
  c u(.p(a), .o(o), .lo(lo));
  initial #1 $display("o=%b lo=%b", o, lo);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"o=1 lo=0001"), "{msgs:?}");
}
