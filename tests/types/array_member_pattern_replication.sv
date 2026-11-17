// IEEE 1800-2023 §10.9.1/§10.10.1: replication covers an entire array
// dimension; §10.9.2: both ordered and named structure members supply it.
// §4.7/§6.8: one process initializes and samples after a settling delay.
`timescale 1ns/1ps
module top;
  parameter N = 3;
  localparam L = 3;
  typedef struct {int a[2];} pair_t;
  typedef struct {int a[1];} one_t;
  typedef struct {int a[3];} three_t;
  typedef struct {int a[3:0];} four_t;
  typedef struct {int a[2][2];} matrix_t;
  typedef struct {logic signed [15:0] a[2];} signed_t;
  typedef struct {logic signed [15:0] a[1];} signed_one_t;
  typedef struct {logic [7:0] a[2];} packed_t;
  typedef struct {logic [3:0] a[2]; bit [3:0] b[2];} states_t;
  typedef struct {int x; int y;} elem_t;
  typedef struct {elem_t a[2];} structs_t;
  typedef struct {real a[2];} real_t;
  typedef struct {string a[2];} string_t;
  typedef struct {logic signed [15:0] a[1];} bit_replication_t;
  bit_replication_t bit_replication;
  pair_t ordered, keyed;
  one_t one;
  three_t literal_count, parameter_count, localparam_count, expression_count;
  four_t several;
  matrix_t nested;
  signed_t signed_elements;
  signed_one_t signed_one;
  packed_t packed_elements;
  states_t states;
  structs_t structs;
  real_t reals;
  string_t strings;
  initial begin
    // §11.4.12.1: explicit bit replication remains one unsigned item.
    bit_replication = '{a:'{{1{8'shfe}}}};
    ordered = '{'{2{7}}};
    keyed = '{a:'{2{7}}};
    one = '{'{1{9}}};
    literal_count = '{'{3{8}}};
    parameter_count = '{a:'{N{8}}};
    localparam_count = '{a:'{L{8}}};
    expression_count = '{'{(1+2){8}}};
    several = '{'{2{3,5}}};
    nested = '{'{2{'{2{7}}}}};
    // §10.9.1/§10.8: every element receives its own target conversion.
    signed_elements = '{'{2{8'shfe}}};
    // §10.9.1/§10.8: count one still converts the signed element.
    signed_one = '{a:'{1{8'sh80}}};
    packed_elements = '{'{2{'1}}};
    states = '{a:'{2{4'bxz10}}, b:'{2{4'bxz10}}};
    structs = '{'{2{'{3,5}}}};
    reals = '{'{2{1.5}}};
    strings = '{a:'{2{"hi"}}};
    #1;
    $display("T|ordered %0d,%0d keyed %0d,%0d one %0d",ordered.a[0],ordered.a[1],keyed.a[0],keyed.a[1],one.a[0]);
    $display("T|literal %0d,%0d,%0d",literal_count.a[0],literal_count.a[1],literal_count.a[2]);
    $display("T|parameter %0d,%0d,%0d",parameter_count.a[0],parameter_count.a[1],parameter_count.a[2]);
    $display("T|localparam %0d,%0d,%0d",localparam_count.a[0],localparam_count.a[1],localparam_count.a[2]);
    $display("T|expression %0d,%0d,%0d",expression_count.a[0],expression_count.a[1],expression_count.a[2]);
    $display("T|several %0d,%0d,%0d,%0d",several.a[3],several.a[2],several.a[1],several.a[0]);
    $display("T|nested %0d,%0d,%0d,%0d",nested.a[0][0],nested.a[0][1],nested.a[1][0],nested.a[1][1]);
    $display("T|signed %h,%h packed %h,%h",signed_elements.a[0],signed_elements.a[1],packed_elements.a[0],packed_elements.a[1]);
    $display("T|states %b,%b/%b,%b",states.a[0],states.a[1],states.b[0],states.b[1]);
    $display("T|structs %0d,%0d/%0d,%0d",structs.a[0].x,structs.a[0].y,structs.a[1].x,structs.a[1].y);
    $display("T|real %0.1f,%0.1f",reals.a[0],reals.a[1]);
    $display("T|string %0s,%0s",strings.a[0],strings.a[1]);
    $display("T|bit replication %h",bit_replication.a[0]);
    $display("T|signed one %h",signed_one.a[0]);
    $finish;
  end
endmodule
