//! IEEE 1800-2023 §13.4, A.2.7: an old-style subroutine port declaration
//! takes a data_type_or_implicit, and a user-defined type may carry packed
//! dimensions (A.2.2.1), so in `input s_t [N-1:0] in;` the type is `s_t` and
//! the port is `in`. The port was taken to be `s_t` with an unpacked
//! dimension, which left `in;` behind as a statement: strict mode then
//! reported the next block item declaration as following a statement
//! (§9.3.1), and a second declarator or packed dimension was a parse error.
//! A port named with an unpacked dimension (`input x [3:0];`) keeps that
//! meaning. Outputs match Verilator 5.046.

use xezim::simulate;

#[test]
fn body_port_of_packed_user_type() {
    let src = r#"
package p;
  typedef struct packed { logic a; logic b; } s_t;
  typedef struct packed { logic a; } t_t;
endpackage
module top;
  import p::*;
  localparam int N = 2;
  function automatic t_t [N-1:0] conv;
    input s_t [N-1:0] in;
    t_t [N-1:0] out;
    begin
      for (int i = 0; i < N; i++) out[i] = '{a: in[i].a};
      conv = out;
    end
  endfunction
  task automatic tk;
    input s_t [1:0][1:0] a, b;
    input var s_t [N-1:0] c;
    output t_t [N-1:0] o;
    t_t [N-1:0] tmp;
    begin
      for (int i = 0; i < N; i++) tmp[i] = '{a: a[1][i].b ^ b[0][i].a ^ c[i].a};
      o = tmp;
    end
  endtask
  function automatic logic [3:0] legacy;
    input x [3:0];
    logic [3:0] y;
    begin
      for (int i = 0; i < 4; i++) y[i] = x[i];
      legacy = y;
    end
  endfunction
  initial begin
    s_t [N-1:0] x;
    t_t [N-1:0] o;
    logic arr [3:0];
    x = 4'b1001;
    tk(8'b01_10_00_00, 8'b00_00_10_00, 4'b00_10, o);
    arr[3] = 1; arr[2] = 0; arr[1] = 1; arr[0] = 1;
    $display("conv=%b", conv(x));
    $display("tk=%b", o);
    $display("legacy=%b", legacy(arr));
    begin : blk
      s_t [N-1:0] v;
      t_t [N-1:0] w;
      v = 4'b0110;
      w = conv(v);
      $display("blk=%b", w);
    end
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("packed user-typed body ports are legal");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    for want in ["conv=10", "tk=01", "legacy=1011", "blk=01"] {
        assert!(lines.contains(&want), "missing {want}: {lines:?}");
    }
}

/// §5.6.1: an escaped identifier is an identifier, so `input s_t \in1 ;` in a
/// subroutine body and `input s_t \pi ;` in a module's port declarations name
/// the ports `in1` and `pi`, with or without packed dimensions on the type.
/// The type lookahead only accepted a plain name after the type, so `s_t`
/// became the port and the escaped name was a parse error or a statement.
#[test]
fn escaped_port_name_after_a_user_type() {
    let src = r#"
typedef struct packed { logic a; logic b; } s_t;
module m (\pi , \pk , y);
  input s_t \pi ;
  input s_t [1:0] \pk ;
  output logic y;
  assign y = \pi .b ^ \pk [1].a;
endmodule
module top;
  function automatic logic f1;
    input s_t \in1 ;
    logic r;
    begin
      r = \in1 .b;
      f1 = r;
    end
  endfunction
  function automatic logic [1:0] f2;
    input s_t [1:0] \in2 ;
    logic [1:0] r;
    begin
      r = {\in2 [1].a, \in2 [0].b};
      f2 = r;
    end
  endfunction
  task automatic tk;
    output s_t \o1 , \o2 ;
    begin
      \o1 = 2'b01;
      \o2 = 2'b10;
    end
  endtask
  logic y0, y1;
  m u0 (.\pi (2'b01), .\pk (4'b0000), .y(y0));
  m u1 (.\pi (2'b01), .\pk (4'b1000), .y(y1));
  initial begin
    s_t a, b;
    tk(a, b);
    #1;
    $display("f1=%b f2=%b tk=%b%b y=%b%b", f1(2'b01), f2(4'b1001), a, b, y0, y1);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("escaped port names after a user type are legal");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"f1=1 f2=11 tk=0110 y=10"), "{lines:?}");
}
