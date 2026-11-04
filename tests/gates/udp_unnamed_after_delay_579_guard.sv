// Guard, not part of the bug: an unnamed instance of a MODULE is still an
// error (IEEE 1800-2017 23.3.2: a module instance needs a name), with or
// without a delay in front. The fix must not turn this into silent acceptance.
// Expected: a compile/elaboration error; the program must not run.
module leaf (output o, input i);
  assign o = i;
endmodule

module t;
  reg i = 1;
  wire o;
  leaf #0 (o, i);
  initial $display("FAIL: an unnamed module instance was accepted");
endmodule
