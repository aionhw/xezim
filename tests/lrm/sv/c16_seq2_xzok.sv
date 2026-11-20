// top: c16_seq2
module c16_seq2;
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
  `CHK(q_star, a |-> ##[*] c)
  `CHK(q_plus, a |-> ##[+] c)
  `CHK(q_repstar, a ##1 b[*] ##1 c |-> 1)
// XZFAIL   `CHK(q_repplus, a ##1 b[+] ##1 c |-> 1)
  `CHK(q_snext, a |-> s_nexttime b)
  `CHK(q_nextn, a |-> nexttime [2] c)
  `CHK(q_ev_range, a |-> eventually [1:3] c)
  `CHK(q_sev_range, a |-> s_eventually [1:3] c)
  `CHK(q_salways, a |-> s_always [0:1] !d)
// XZFAIL   `CHK(q_until_with, a |=> b until_with c)
  `CHK(q_suntil, a |=> b s_until c)
  `CHK(q_strong, a |-> strong(##[1:2] c))
  `CHK(q_weak, a |-> weak(##[1:2] c))
// XZFAIL   `CHK(q_accept, accept_on(d) a |=> c)
// XZFAIL   `CHK(q_reject, reject_on(d) a |=> c)
// XZFAIL   `CHK(q_sync_accept, sync_accept_on(d) a |=> c)
// XZFAIL   `CHK(q_sync_reject, sync_reject_on(d) a |=> c)
  `CHK(q_or_prop, (a |-> b) or (a |-> c))
  `CHK(q_and_prop, (a |-> ##1 c) and (b |-> 1))
  `CHK(q_not_prop, not (a |-> ##1 a))
// XZFAIL   `CHK(q_followed_ov, a #-# b)
// XZFAIL   `CHK(q_followed_nov, a #=# c)
  `CHK(q_seq_and_len, a |-> (##1 b ##1 c) and (##2 c))
  `CHK(q_onehot, $onehot({a,b}) |-> !c)
  `CHK(q_isunk, !$isunknown({a,b,c,d}))
  `CHK(q_ended_prop, a |-> ##[0:2] (b ##1 c))
endmodule
