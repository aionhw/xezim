// IEEE 1800-2023 §6.7.1 permits fixed arrays of valid net members; §6.6.5 preserves implicit pulls against z; §20.9 $isunknown is false for fully resolved 0/1. §6.6.5: "A tri1 net is equivalent to a wire net with a continuous 1 value of pull strength driving it."
// Complete patterns driving fixed array members with z retain tri0/tri1 pulls; assert bit and unknown flags, never wrong full-element widths.
`timescale 1ns/1ps
// §6.6.5, §6.7.1: all elements retain pull values when all drivers supply z.
typedef struct {logic [7:0] value[2];} S;
module top;
 logic [7:0] source[2]='{default:'z};
 tri0 S low;
 tri1 S high;
 assign low='{source}; assign high='{source};
 initial begin #2;
  $display("T|array-pulls %b %b unknown %0d %0d",low.value[0][0],high.value[0][0],$isunknown(low.value[0]),$isunknown(high.value[0]));
  $finish;
 end
endmodule
