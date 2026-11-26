int uv = 4;
task automatic bumpu(); uv += 10; endtask
module top; libv a(); liby b(); initial begin #3; $display("T|top uv=%0d", uv); $finish; end endmodule
