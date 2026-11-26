int cnt = 0;
int log [$];
task automatic note(int id); cnt++; log.push_back(id); endtask
interface ifc; int x; initial begin #1; note(1); x = cnt; end endinterface
program prg; initial begin #2; note(2); $display("T|prg cnt=%0d", cnt); #20; end endprogram
module leaf #(parameter int ID = 0); initial begin #3; note(ID); end endmodule
module l2; leaf #(30) u(); endmodule
module l1; l2 u(); endmodule
module arrm; leaf #(40) u [1:0] (); endmodule
module gen; genvar i; for (i = 0; i < 2; i++) begin : g leaf #(50 + i) u(); end initial begin #4; cnt += 200; end endmodule
module carrier_plain; logic q; endmodule
module topa; ifc i(); prg p(); l1 a(); arrm b(); gen c(); carrier_plain d();
  initial begin #5; $display("T|topa cnt=%0d log=%p i.x=%0d", cnt, log, i.x); end
endmodule
module topb; always @(cnt) $display("T|topb cnt=%0d at %0t", cnt, $time);
  initial begin #6; $display("T|topb end cnt=%0d n=%0d", cnt, log.size()); $finish; end
endmodule
