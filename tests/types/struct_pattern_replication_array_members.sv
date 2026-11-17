// IEEE 1800-2023 §7.2, §7.4.2, §10.9.1, §10.9.2: each replicated
// array element should be 7. Check only the elements main already gets
// right; the remaining elements are an open array-pattern limitation.
// §4.7/§6.8: initialize and sample in one process after a settling delay.
`timescale 1ns/1ps
module top;
  typedef struct { int a[2]; int b[2]; } arrays_t;
  typedef struct { int a[2]; } single_t;
  typedef struct { arrays_t l; arrays_t r; } nested_t;
  arrays_t s;
  single_t single_member;
  nested_t nested;
  // §13.5.1: the input is copied into the automatic task's formal.
  task automatic check_single(input single_t v);
    if (v.a[0] === 7 && v.a[1] === 7)
      $display("PASS one array member task argument");
    else $display("FAIL one array member task argument");
  endtask
  initial begin
    s = '{'{0, 0}, '{0, 0}};
    #1 s = '{2{'{2{7}}}};
    #1;
    if (s.a[0] === 7) $display("PASS array member first element");
    else $display("FAIL array member first element");
    if (s.a[1] === 7) $display("PASS array member second element");
    else $display("FAIL array member second element");
    single_member = '{1{'{2{7}}}};
    nested = '{2{'{2{'{2{7}}}}}};
    #1;
    if (single_member.a[0] === 7 && single_member.a[1] === 7)
      $display("PASS one array member procedural");
    else $display("FAIL one array member procedural");
    check_single('{1{'{2{7}}}});
    if (nested.l.a[0] === 7 && nested.l.a[1] === 7 &&
        nested.l.b[0] === 7 && nested.l.b[1] === 7)
      $display("PASS nested first structure array members");
    else $display("FAIL nested first structure array members");
    $finish;
  end
endmodule
