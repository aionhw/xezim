// top: c16_seq
module c16_seq;
  initial $assertvacuousoff(0);
  bit clk; bit a, b, c, d, rst;
  int cyc;
  always #5 clk = ~clk;
  always @(posedge clk) cyc <= cyc + 1;
  // stimulus table
  bit [3:0] va [0:23] = '{4'b0001,4'b0011,4'b0110,4'b0100,4'b0001,4'b0000,4'b0010,4'b0001,4'b0011,4'b0101,4'b1000,4'b0001,
                          4'b0010,4'b0010,4'b0100,4'b0001,4'b1001,4'b0011,4'b0000,4'b0110,4'b0001,4'b0010,4'b0100,4'b0000};
  initial begin
    for (int i = 0; i < 24; i++) begin @(negedge clk); {d,c,b,a} = va[i]; end
    @(negedge clk) {d,c,b,a} = 0; repeat(8) @(negedge clk);
    $finish;
  end
  `define CHK(nm, prop) nm: assert property (@(posedge clk) prop) $display("T|%s|P t=%0t", `"nm`", $time); else $display("T|%s|F t=%0t", `"nm`", $time);
  `CHK(p_impl_ov, a |-> b)
  `CHK(p_impl_nov, a |=> c)
  `CHK(p_delay2, a |-> ##2 c)
  `CHK(p_range, a |-> ##[1:3] c)
  `CHK(p_rep, b[*2] |=> c)
  `CHK(p_rep_range, a ##1 b[*1:2] ##1 c)
  `CHK(p_goto, a |-> ##1 c[->1] ##1 a)
  `CHK(p_nonc, a |=> b[=1] ##1 a)
  `CHK(p_and, (a ##1 b) and (a ##2 c))
  `CHK(p_or, a |-> (##1 b) or (##2 c))
  `CHK(p_intersect, a |-> (##[1:3] c) intersect (b[*2]))
  `CHK(p_within, a |-> (c[->1]) within (##[0:4] a))
  `CHK(p_throughout, a |-> (!d) throughout (##2 c))
  `CHK(p_fm, a |-> first_match(##[1:3] c))
  `CHK(p_not, not (a ##1 a))
  `CHK(p_ifelse, a |-> if (b) ##1 c else ##1 !c)
  `CHK(p_sev, a |-> s_eventually d)
  `CHK(p_until, a |=> b until c)
// XZFAIL   `CHK(p_suntil_with, a |=> b s_until_with c)
  `CHK(p_next, a |-> nexttime b)
// XZFAIL   `CHK(p_always, a |-> always [0:1] !d)
// XZFAIL   `CHK(p_impl_prop, a implies b)
  `CHK(p_iff, a iff b)
  `CHK(p_zero_rep, a ##1 b[*0:1] ##1 c |-> 1)
  `CHK(p_unb, a |-> ##[1:$] d)
  `CHK(p_disable, disable iff (d) a |=> c)
// XZFAIL   `CHK(p_case, case (b) 1'b1: a |-> 1; default: a |-> ##1 c; endcase)
  cov1: cover property (@(posedge clk) a ##1 b) $display("T|cov1|C t=%0t", $time);
  cov2: cover property (@(posedge clk) a ##[1:2] c) $display("T|cov2|C t=%0t", $time);
endmodule
