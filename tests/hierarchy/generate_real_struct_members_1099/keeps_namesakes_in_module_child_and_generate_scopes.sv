// IEEE 1800-2023 §23.6: "shall have a unique hierarchical path name"; §6.8 default four-state variable is x; §6.7.1: net default "shall be the value z".
// Member storage must keep generated variables, child nets and parent nets independent despite identical local names.
`timescale 1ns/1ps
typedef struct {logic [7:0] value;} S;
module child;wire S a;assign a='{8'h81};endmodule
module top;
 wire S a;child c();wire S v;assign v='{8'h92};
 if(1)begin:g S v;initial #1 $display("T|generated %h",v.value);end
 initial begin #2;$display("T|parent %h",a.value);$finish;end
endmodule
