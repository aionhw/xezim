// IEEE 1800-2023 §§7.2,10.9,6.24.1 preserve z in patterns/casts; §11.4.11 selects off; §6.6.5 supplies pull 1; §28.12.1 pull beats weak 0, giving zz/ff/ff/ff/ff. §6.6.5: "A tri1 net is equivalent to a wire net with a continuous 1 value of pull strength driving it."
// Conditional/default-pattern/cast/strength wrappers must not become zero drivers; only off/pull fields are main-correct.
`timescale 1ns/1ps
typedef struct {logic [7:0] value;} S;
module top;
 S data='{8'h9a},off='{8'hzz};logic en=0;
 wire S w;tri1 S h;tri1 S pat,casted,weak_net;
 assign w=en ? data : off;assign h=en ? data : off;
 assign pat='{default:'z};assign casted=S'('{8'hzz});
 assign (weak1,weak0) weak_net='{8'h00};
 initial begin #2;$display("T|off %h %h %h %h %h",w.value,h.value,pat.value,casted.value,weak_net.value);$finish;end
endmodule
