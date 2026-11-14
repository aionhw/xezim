// IEEE 1800-2023 §6.19, §7.4.1: an enum's packed base retains its shape.
// §10.3.2 permits one continuous driver of a variable; §10.3.3 chooses
// vector rise for x/partly z, fall for nonzero -> zero and off for all z.
// Scalars use §28.16's minimum for x. One delay applies to all transitions;
// two delays imply off=min(rise,fall). Casts are explicit (§6.24.1).
// §6.7.1: only four-state types are used for built-in nets. Variables
// use only a single delay (Syntax 10-1 / §A.6.1).
// Typedefs (§6.18), packages (§26), instances (§23.3) and named generate
// blocks (§27.4, §27.5) must preserve the shape. Samples are between events
// (§4.7); each stimulus is held long enough to settle before the next one.
`timescale 1ns/1ps
`define FOUR(T, C, W, V, N) \
  if (1) begin : N \
    logic [W-1:0] a; \
    wire T w1, w2, w3; \
    T v1; \
    assign #4 w1 = C'(a); \
    assign #(4,2) w2 = C'(a); \
    assign #(4,6,2) w3 = C'(a); \
    assign #4 v1 = C'(a); \
    task automatic sample(input logic [W-1:0] e1, input logic [W-1:0] e2, input logic [W-1:0] e3); \
      if (w1 !== e1 || w2 !== e2 || w3 !== e3 || \
          v1 !== e1) \
        $display("FAIL %m t=%0t got=%b/%b/%b variable=%b expected=%b/%b/%b", \
          $time, w1,w2,w3,v1,e1,e2,e3); \
    endtask \
    initial begin \
      a = '1; #9 sample('1,'1,'1); \
      #1 a = 'x; #1 sample('1,'1,'1); \
      #2 sample('1,V ? '1 : 'x,V ? '1 : 'x); \
      #2 sample('x,'x,'x); \
      #5 a = '1; #9 sample('1,'1,'1); \
      #1 a = 'z; #1 sample('1,'1,'1); \
      #2 sample('1,'z,'z); #2 sample('z,'z,'z); \
      #5 a = '1; #10 a = '0; \
      #3 sample('1,'0,'1); #2 sample('0,'0,'1); #2 sample('0,'0,'0); \
      #3 a = '1; #3 sample('0,'0,'0); #2 sample('1,'1,'1); \
      if (W > 1) begin \
        #5 a = 'bz01; #3 sample('1,'1,'1); #2 sample('bz01,'bz01,'bz01); \
      end \
      $display("PASS %m"); \
    end \
  end
// Two-state enum casts map x/z to zero (§6.11.2, §6.24.1). Use variables:
// two-state bases are not legal built-in net data types (§6.7.1).
`define TWO(T, W, N) \
  if (1) begin : N \
    logic [W-1:0] a; \
    T v; \
    assign #4 v = T'(a); \
    task automatic sample(input logic [W-1:0] e); \
      if (v !== e) $display("FAIL %m got=%b expected=%b",v,e); \
    endtask \
    initial begin \
      a = '1; #9 sample('1); \
      #1 a = 'x; #3 sample('1); #2 sample('0); \
      #5 a = '1; #10 a = 'z; #3 sample('1); #2 sample('0); \
      #5 a = '1; #10 a = '0; #3 sample('1); #2 sample('0); \
      $display("PASS %m"); \
    end \
  end
package shapes;
  typedef enum logic [0:0] {P0=1'b0,P1=1'b1} E;
  typedef enum logic {S0=1'b0,S1=1'b1} S;
endpackage
module checks;
  typedef enum logic [0:0] {L0=1'b0,L1=1'b1} E;
  typedef logic [0:0] Base;
  typedef enum Base {BA0=1'b0,BA1=1'b1} BaseAlias;
  typedef enum logic {A0=1'b0,A1=1'b1} [0:0] EnumArray;
  typedef E Alias;
  typedef Alias Alias2;
  typedef enum logic {S0=1'b0,S1=1'b1} Scalar;
  typedef enum logic signed [0:0] {N=1'sb1,Z=1'sb0} Signed;
  typedef enum logic signed {SN=1'sb1,SZ=1'sb0} SignedScalar;
  typedef enum logic [3:0] {Q0=4'b0,Q1=4'b1111} Wide;
  typedef enum logic signed [3:0] {WN=-4'sd1,WZ=4'sd0} SignedWide;
  typedef enum integer {I0=0,I1=-1} Integer;
  typedef enum bit [0:0] {B0=1'b0,B1=1'b1} Bit;
  typedef enum bit signed [0:0] {BN=1'sb1,BZ=1'sb0} SignedBit;
  typedef enum bit {SB0=1'b0,SB1=1'b1} ScalarBit;
  typedef enum int {J0=0,J1=-1} Int;
  typedef enum int unsigned {U0=0,U1=32'hffffffff} UInt;
  typedef enum {D0=0,D1=-1} Default;
  typedef logic [0:0] V1;
  typedef logic PlainScalar;
  `FOUR(E,E,1,1,enum_vector)
  `FOUR(Alias2,Alias2,1,1,alias_vector)
  `FOUR(BaseAlias,BaseAlias,1,1,base_alias_vector)
  `FOUR(EnumArray,EnumArray,1,1,enum_array_vector)
  `FOUR(Scalar,Scalar,1,0,enum_scalar)
  `FOUR(Signed,Signed,1,1,signed_vector)
  `FOUR(SignedScalar,SignedScalar,1,0,signed_scalar)
  `FOUR(Wide,Wide,4,1,wide_vector)
  `FOUR(SignedWide,SignedWide,4,1,signed_wide)
  `FOUR(Integer,Integer,32,1,integer_vector)
  `FOUR(shapes::E,shapes::E,1,1,package_vector)
  `FOUR(shapes::S,shapes::S,1,0,package_scalar)
  `FOUR(V1,V1,1,1,typedef_guard)
  `FOUR(logic [0:0],V1,1,1,packed_guard)
  `FOUR(PlainScalar,PlainScalar,1,0,scalar_guard)
  `TWO(Bit,1,bit_vector)
  `TWO(SignedBit,1,signed_bit)
  `TWO(ScalarBit,1,scalar_bit)
  `TWO(Int,32,int_vector)
  `TWO(UInt,32,unsigned_int)
  `TWO(Default,32,default_int)
  if (1) begin : local_scope
    typedef enum logic [0:0] {G0=1'b0,G1=1'b1} E;
    `FOUR(E,E,1,1,generated_vector)
  end
  for (genvar i=0; i<2; i++) begin : iterations
    typedef enum logic {G0=1'b0,G1=1'b1} E;
    `FOUR(E,E,1,0,generated_scalar)
  end
endmodule
module top;
  checks child();
  typedef enum logic [0:0] {T0=1'b0,T1=1'b1} E;
  `FOUR(E,E,1,1,top_vector)
  initial #100 $finish;
endmodule
`undef FOUR
`undef TWO
