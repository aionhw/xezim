// §13.3 automatic task inputs; §11.4.6 case equality checks x/z.
`timescale 1ns/1ps
// IEEE 1800-2023: delayed tristate primitives (§28.6 bufif/notif, §28.7 MOS)
// that share a net with a second driver. §28.16: "When two delays are
// given, the first delay shall specify the rise delay, and the second delay
// shall specify the fall delay. The delay when the signal changes to high
// impedance or to unknown shall be the lesser of the two delay values."
// The second driver is a constant z, so the net shows the primitive's own
// output (§28.12: z yields). Inputs change at t=1, 11, 21, 31; samples are
// away from update times (§4.7, §6.8).
module top;
  logic d, en;
  wire bm, nm, mm;
  bufif1 #(1,5) b1(bm, d, en);
  assign #(1,1) bm = 1'bz;
  notif1 #(1,5) n1(nm, d, en);       // output is ~d
  assign #(1,1) nm = 1'bz;
  nmos   #(1,5) m1(mm, d, en);
  assign #(1,1) mm = 1'bz;
  task automatic check(input string n, input logic got, input logic exp);
    if (got === exp) $display("PASS %s t=%0.1f got=%b", n, $realtime, got);
    else $display("FAIL %s t=%0.1f got=%b expected=%b", n, $realtime, got, exp);
  endtask
  initial begin
    #1 d = 1; en = 1;            // bufif/nmos x->1 (rise 1), notif x->0 (fall 5)
    #2 check("bufif_rise", bm, 1'b1); check("nmos_rise", mm, 1'b1);
       check("notif_fall_early", nm, 1'bx);
    #3.5 check("notif_fall", nm, 1'b0);              // t=6.5
    #4.5 d = 0;                    // t=11: bufif/nmos 1->0 (fall 5), notif 0->1
    #4 check("bufif_fall_early", bm, 1'b1);        // t=15
       check("nmos_fall_early", mm, 1'b1);
       check("notif_rise", nm, 1'b1);
    #2 check("bufif_fall", bm, 1'b0);              // t=17
       check("nmos_fall", mm, 1'b0);
    #4 en = 0;                   // t=21: all -> z (turn-off min(1,5) = 1)
    #3 check("bufif_off", bm, 1'bz); check("notif_off", nm, 1'bz);
       check("nmos_off", mm, 1'bz);                // t=24
    $display("DONE");
    $finish;
  end
endmodule
