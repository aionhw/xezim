// one compilation-unit variable, written and read from several scopes
int shared_value = 7;
int arr [4];
string label = "init";
function automatic void bump(int by); shared_value += by; endfunction
class C; function void set(int v); shared_value = v; endfunction endclass
package p; int pv = 1; endpackage
module writer;
  initial begin #1; $unit::shared_value = 23; arr[2] = 5; label = "w"; p::pv = 9; $display("T|writer %0d", shared_value); end
endmodule
module reader #(parameter int ID = 0);
  initial begin #2; $display("T|reader%0d bare=%0d q=%0d arr2=%0d label=%s pv=%0d", ID, shared_value, $unit::shared_value, arr[2], label, p::pv); end
  always @(shared_value) $display("T|reader%0d saw change to %0d at %0t", ID, shared_value, $time);
endmodule
module fwriter; initial begin #3; bump(100); $display("T|fwriter %0d", shared_value); end endmodule
module cwriter; C c; initial begin #5; c = new; c.set(-4); $display("T|cwriter %0d", shared_value); end endmodule
module shadow; int shared_value = 1; initial begin #4; $display("T|shadow local=%0d unit=%0d", shared_value, $unit::shared_value); $unit::shared_value++; end endmodule
module top;
  writer a(); reader #(1) b(); reader #(2) c(); fwriter d(); shadow e(); cwriter f();
  wire [31:0] w = shared_value;
  initial begin #6; $display("T|top %0d w=%0d", shared_value, w); $finish; end
endmodule
