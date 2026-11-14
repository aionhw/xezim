// IEEE 1800-2023 §10.9.2: replication removes the inner braces and each
// item takes its member's type. §6.6.7 permits these user-defined nettypes;
// each resolved net here has one driver, returned by its resolver.
// §6.11.2: x/z bits become zero in the 2-state bit members.
// §4.7 / §6.8: initialize explicitly and sample between stimulus times.
`timescale 1ns/1ps
typedef struct { logic [7:0] a; bit [15:0] b; } pair_t;
typedef struct { logic [7:0] a; bit [15:0] b;
                 logic [3:0] c; bit [31:0] d; } four_t;
typedef struct { pair_t left; pair_t right; } nested_t;
function automatic pair_t resolve_pair(input pair_t drivers[]);
  return drivers[0];
endfunction
function automatic four_t resolve_four(input four_t drivers[]);
  return drivers[0];
endfunction
nettype pair_t pair_net;
nettype pair_t pair_resolved with resolve_pair;
nettype four_t four_net;
nettype four_t four_resolved with resolve_four;
nettype nested_t nested_net;
module top;
  logic [7:0] x;
  bit [15:0] y;
  pair_t pc, pp, pnb, pcomb, ordered, named, side;
  pair_net pu;
  pair_resolved pr;
  four_t fc, fp, ones;
  four_net fu;
  four_resolved fr;
  nested_t nc, np;
  nested_net nu;
  int calls;
  function automatic int value_with_effect();
    calls++;
    return 11;
  endfunction
  always_comb pcomb = '{2{x}};
  assign pc = '{2{x}};
  assign pu = '{2{x}};
  assign pr = '{2{x}};
  assign fc = '{2{x, y}};
  assign fu = '{2{x, y}};
  assign fr = '{2{x, y}};
  assign nc = '{2{'{2{x}}}};
  assign nu = '{2{'{2{x}}}};
  assign ordered = '{x, y};
  assign named = '{b:y, a:x};
  task automatic check_pair(input string label, input pair_t p);
    if (p.a === x && p.b === 16'(int'(x))) $display("PASS %s", label);
    else $display("FAIL %s %h %h", label, p.a, p.b);
  endtask
  task automatic check_four(input string label, input four_t f);
    if (f.a === x && f.b === y && f.c === x[3:0] && f.d === {16'b0,y})
      $display("PASS %s", label);
    else $display("FAIL %s %h %h %h %h", label, f.a, f.b, f.c, f.d);
  endtask
  task automatic check_nested(input string label, input nested_t n);
    if (n.left.a === x && n.left.b === 16'(int'(x)) &&
        n.right.a === x && n.right.b === 16'(int'(x)))
      $display("PASS %s", label);
    else $display("FAIL %s", label);
  endtask
  initial begin
    calls = 0;
    x = 7; y = 23;
    repeat (2) begin
      #1;
      pnb <= '{2{x}};
      pp = '{2{x}};
      fp = '{2{x, y}};
      np = '{2{'{2{x}}}};
      #1;
      check_pair("pair continuous variable", pc);
      check_pair("pair continuous unresolved", pu);
      check_pair("pair continuous resolved", pr);
      check_pair("pair procedural", pp);
      check_pair("pair nonblocking", pnb);
      check_pair("pair always_comb", pcomb);
      check_four("four continuous variable", fc);
      check_four("four continuous unresolved", fu);
      check_four("four continuous resolved", fr);
      check_four("four procedural", fp);
      check_nested("nested continuous variable", nc);
      check_nested("nested continuous unresolved", nu);
      check_nested("nested procedural", np);
      if (ordered.a === x && ordered.b === y && named.a === x && named.b === y)
        $display("PASS ordered named guards");
      else $display("FAIL ordered named guards");
      #1 x = 8'b10xz0101; y = 16'h2345;
    end
    // Unbased unsized '1 fills each member, irrespective of member width (§5.7.1).
    ones = '{2{'1, '1}};
    if (ones.a === 8'hff && ones.b === 16'hffff &&
        ones.c === 4'hf && ones.d === 32'hffffffff) $display("PASS member context");
    else $display("FAIL member context");
    // §10.9.2 leaves the evaluation count of a side effect undefined.
    // Check only the fixed return value; never assert or print the call count.
    side = '{2{value_with_effect()}};
    if (side.a === 11 && side.b === 11) $display("PASS side effect values");
    else $display("FAIL side effect values");
    $finish;
  end
endmodule
