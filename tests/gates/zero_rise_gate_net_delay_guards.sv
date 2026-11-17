// IEEE 1800-2023 §28.16/§10.3.3: separate gate/assign and net stages.
// §23.3.3.7 output collapsing; §28.3.5/§28.3.6 primitive arrays;
// §28.12 equal-strength driver resolution. Samples avoid updates (§4.7).
`timescale 1ns/1ps
module child(input a, output w);
  buf #(0,5) g(w,a);
endmodule
module top;
  logic a,en;
  wire #(2,3) none,one,both,zero_fall,assigned,collapsed,two;
  wire plain;
  wire #2 single;
  wire #(2,3,4) triple,off;
  wire [1:0] #(2,3) arrayed;
  // §28.12/§28.15: retain main-correct samples on implicit-driver nets.
  tri0 #(2,3) pulled;
  supply0 #(2,3) powered;
  buf (weak0,weak1) #(0,5) gpull(pulled,a);
  buf #(0,5) gsupply(powered,a);
  // §5.6.1 permits $ in user identifiers; staging must preserve them.
  wire #(2,3) collision;
  wire collision$netdrv;
  assign collision$netdrv=1'b1;
  buf #(0,5) gcollision(collision,a);
  buf gn(none,a);
  buf #1 go(one,a);
  buf #(1,5) gb(both,a);
  buf #(5,0) gf(zero_fall,a);
  buf #(0,5) gp(plain,a),gs(single,a),gt(triple,a);
  assign #(0,5) assigned=a;
  child c(a,collapsed);
  buf #(0,5) g2(two,a),h2(two,a);
  buf #(0,5) ga[1:0](arrayed,{a,a});
  // A two-delay primitive turns off with min(1,5); the NET uses #4.
  bufif1 #(1,5) gz(off,a,en);
  initial begin
    #1 a=1; en=1;
    #20 a=0;
    #20 a=1;
    #20 en=0;
  end
  initial begin
    #24.5 $display("T|early=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    #2 $display("T|gate_fell=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    $display("T|implicit_drivers=%b%b",pulled,powered);
    $display("T|generated_name=%b",collision$netdrv);
    #2 $display("T|single_fell=%b",{single,triple,assigned,collapsed,two,arrayed,off});
    #1 $display("T|net_fell=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    #13 $display("T|rise_pending=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    #1 $display("T|rise=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    #5 $display("T|settled=%b",{none,one,both,zero_fall,assigned,collapsed,two,plain,single,triple,arrayed,off});
    #15 $display("T|off_pending=%b",off);
    #1 $display("T|off_pending=%b",off);
    #2 $display("T|off=%b",off);
    $finish;
  end
endmodule
