// IEEE 1800-2023 §§28.3.2,28.10,28.12.4: equal pull 0/1 on wor resolve to 1; §10.11 gives both names the same bit; §23.3.3.3 permits the selected actual. §10.11: "same physical net".
// Explicit primitive-strength provenance must survive selected actuals and nested child ports.
`timescale 1ns/1ps
module child(output wor o,input d);buf (pull1,pull0) g(o,d);endmodule
module wrap(output wor o,input d);child u(o,d);endmodule
module top;
 logic d=1;wor [0:0] a,b;alias a=b;wrap u(a[0],d);pullup(a);
 initial begin #1;$display("T|enabled %b%b",a,b);$finish;end
endmodule
