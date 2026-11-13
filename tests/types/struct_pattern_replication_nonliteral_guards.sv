// IEEE 1800-2023 §10.9.1: array replication retains element order.
// §10.9.2: packed structure patterns already expand replication.
`timescale 1ns/1ps
module top;
  localparam int N = 2;
  typedef struct packed { logic [7:0] a; bit [15:0] b;
                          logic [3:0] c; bit [31:0] d; } packed_t;
  typedef struct { logic [7:0] a; } single_t;
  single_t sc, sp;
  int x, y;
  int ac[4], ap[4], two[2];
  packed_t pc, pp;
  assign sc = '{{2{4'h5}}};
  assign ac = '{N{x, y}};
  assign pc = '{N{x, y}};
  initial begin
    x = 7; y = 23;
    #2;
    sp = '{{2{4'h5}}};
    if (sc.a === 8'h55 && sp.a === 8'h55)
      $display("PASS explicit bit replication"); else $display("FAIL explicit bit replication");
    ap = '{N{x, y}};
    two = '{(1 + 1){x}};
    pp = '{N{x, y}};
    if (ac[0] === 7 && ac[1] === 23 && ac[2] === 7 && ac[3] === 23)
      $display("PASS array continuous"); else $display("FAIL array continuous");
    if (ap[0] === 7 && ap[1] === 23 && ap[2] === 7 && ap[3] === 23 &&
        two[0] === 7 && two[1] === 7)
      $display("PASS array procedural"); else $display("FAIL array procedural");
    if (pc.a === 7 && pc.b === 23 && pc.c === 7 && pc.d === 23 && pp === pc)
      $display("PASS packed guard"); else $display("FAIL packed guard");
    $finish;
  end
endmodule
