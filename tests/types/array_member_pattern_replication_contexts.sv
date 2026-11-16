// IEEE 1800-2023 §10.9.1/§10.10.1/§10.9.2: array-member replication
// in assignment contexts, with guards for existing positional/default paths.
`timescale 1ns/1ps
// §23.3.3: the connection is an assignment-like context.
typedef struct {int a[2];} pair_t;
module consumer(input pair_t s);
  initial begin #1; $display("T|port %0d,%0d",s.a[0],s.a[1]); end
endmodule
module top;
  typedef struct {int a[2]; int b;} with_default_t;
  // §6.8: static initialization precedes initial processes.
  pair_t initialized = '{a:'{2{7}}};
  pair_t continuous_value, nba, returned, plain, defaults;
  pair_t sarr[2];
  with_default_t sibling_default;
  // §10.3.2: a variable driven by a single continuous assignment.
  assign continuous_value = '{a:'{2{7}}};
  // §13.4: returned patterns receive the function result's type.
  function automatic pair_t make(); return '{a:'{2{7}}}; endfunction
  // §13.5.1: replicate before copying the input argument.
  function automatic int total(input pair_t v); return v.a[0]+v.a[1]; endfunction
  consumer u(.s('{a:'{2{7}}}));
  initial begin
    nba <= '{a:'{2{7}}};
    returned = make();
    plain = '{'{7,7}};
    defaults = '{a:'{default:7}};
    sibling_default = '{a:'{2{7}},default:9};
    // §10.9.1: a direct element-member target supplies the array context.
    sarr[1].a = '{2{7}};
    // §4.7/§6.8: sample away from initialization and NBA updates.
    #2;
    $display("T|declaration %0d,%0d",initialized.a[0],initialized.a[1]);
    $display("T|continuous %0d,%0d",continuous_value.a[0],continuous_value.a[1]);
    $display("T|nba %0d,%0d",nba.a[0],nba.a[1]);
    $display("T|return %0d,%0d",returned.a[0],returned.a[1]);
    $display("T|argument %0d",total('{a:'{2{7}}}));
    $display("T|plain %0d,%0d default %0d,%0d",plain.a[0],plain.a[1],defaults.a[0],defaults.a[1]);
    $display("T|sibling default %0d,%0d/%0d",sibling_default.a[0],sibling_default.a[1],sibling_default.b);
    $display("T|element member %0d,%0d",sarr[1].a[0],sarr[1].a[1]);
    $finish;
  end
endmodule
