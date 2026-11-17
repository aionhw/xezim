// IEEE 1800-2023 §§6.12,20.5 binary32/64 exactly represent 1023*2^52=4607182418800017408; §6.18 preserves typedef identity; §5.6.1 escaped identifiers include printable non-whitespace characters. §6.12: "The shortreal data type is the same as a C float."
// Escaped brackets and dots are identifier characters, not selects; preserve the declined shortreal fallback value.
`timescale 1ns/1ps
// §5.6.1: brackets and dots in escaped identifiers are literal characters.
// §6.12, §6.18, §7.2: the generated shortreal member holds the exact large value.
typedef real R;
typedef struct {R \v[0] ;} A;
typedef struct {shortreal \v[0] ;} B;
module top;
 A \a[0] ;
 assign \a[0] ='{1.0};
 if(1) begin:g
  B \a[0] ;
  assign \a[0] ='{4607182418800017408.0};
 end
 shortreal reference_value=4607182418800017408.0;
 initial begin #2; $display("T|short %0.1f reference %0.1f",g.\a[0] .\v[0] ,reference_value); $finish; end
endmodule
