//! #984 (C, F): §23.2.2.1, §7.2, §7.4.1: a non-ANSI port of a packed struct
//! type records its members' packed shapes, as an ANSI port does, so
//! `arg.vec[1]` of a `logic [3:0][7:0] vec` member selects a byte on the
//! port itself (an output port written in the module) and in a function or
//! task called with it whose formal has the port's name. It was a one-bit
//! select: `vec[1] = 8'hFF` read 0 and `vec[0] = 8'h02` read 1. Expected
//! lines are the reference simulator's.

use xezim::simulate;

#[test]
fn struct_port_members_select_elements() {
    let src = r#"
package p;
  typedef struct packed { logic [3:0][7:0] vec; } s_t;
  function automatic logic [7:0] lookup(input s_t arg, input logic [1:0] sel);
    lookup = arg.vec[sel];
  endfunction
  task automatic tlookup(input s_t arg, input logic [1:0] sel, output logic [7:0] o);
    o = arg.vec[sel];
  endtask
endpackage
module dut (out, tout, arg);
  input p::s_t arg;
  output logic [7:0] out, tout;
  import p::*;
  always_comb out = lookup(arg, 2'd1);
  always_comb tlookup(arg, 2'd1, tout);
endmodule
module dut_local (out, arg);
  typedef struct packed { logic [3:0][7:0] vec; } l_t;
  input l_t arg;
  output logic [7:0] out;
  function automatic logic [7:0] llut(input l_t arg, input logic [1:0] sel);
    llut = arg.vec[sel];
  endfunction
  always_comb out = llut(arg, 2'd1);
endmodule
module dut_ansi (output logic [7:0] out, input p::s_t arg);
  import p::*;
  always_comb out = lookup(arg, 2'd1);
endmodule
module dut_out (arg);
  output p::s_t arg;
  always_comb begin arg = '0; arg.vec[1] = 8'h5A; end
endmodule
module top;
  p::s_t s;
  wire [7:0] w1, t1, w2, w3;
  wire [31:0] w4;
  dut u1 (.out(w1), .tout(t1), .arg(s));
  dut_local u2 (.out(w2), .arg(s));
  dut_ansi u3 (.out(w3), .arg(s));
  dut_out u4 (.arg(w4));
  initial begin
    s = '0; s.vec[1] = 8'hFF;
    #1 $display("C1 fn=%h task=%h local=%h ansi=%h out=%h", w1, t1, w2, w3, w4);
    s = '0; s.vec[0] = 8'h02;
    #1 $display("C2 fn=%h task=%h local=%h ansi=%h", w1, t1, w2, w3);
    s = '0; s.vec[1] = 8'h05;
    #1 $display("C3 fn=%h task=%h local=%h ansi=%h", w1, t1, w2, w3);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "C1 fn=ff task=ff local=ff ansi=ff out=00005a00",
            "C2 fn=00 task=00 local=00 ansi=00",
            "C3 fn=05 task=05 local=05 ansi=05",
        ]
    );
}
