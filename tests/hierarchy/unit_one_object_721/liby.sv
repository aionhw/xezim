module liby; always @(uv) $display("T|liby saw %0d", uv); initial begin #2; bumpu(); end endmodule
