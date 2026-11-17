// IEEE 1800-2023 §§6.12,20.5 binary32/64 exactly represent 1023*2^52=4607182418800017408; §6.18 preserves typedef identity; §5.6.1 escaped identifiers include printable non-whitespace characters. §6.12: "The shortreal data type is the same as a C float."
// A declined shortreal object must retain its main-correct value despite another structure sharing a real alias/member name.
`timescale 1ns/1ps
typedef real R;
typedef struct {R v;} A;
typedef struct {shortreal v;} B;
module top;
 A a;
 assign a='{1.0};
 if(1) begin:g
  B a;
  assign a='{4607182418800017408.0};
 end
 shortreal reference_value=4607182418800017408.0;
 initial begin #2; $display("T|short %0.1f reference %0.1f",g.a.v,reference_value); $finish; end
endmodule
