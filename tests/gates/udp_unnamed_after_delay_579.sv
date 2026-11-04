// IEEE 1800-2017 29.3 (and A.5.4):
//   udp_instantiation ::= udp_identifier [ drive_strength ] [ delay2 ]
//                         udp_instance { , udp_instance } ;
//   udp_instance      ::= [ name_of_instance ] ( output_terminal , input_terminal {...} )
// so the instance name is optional with or without a delay or a drive strength,
// and A.2.2.3 makes a bare delay_value a single primary (#0, #1, #1.5).
// Every check below should print PASS.
`timescale 1ns/1ns
`define DELAY0 #0

primitive my_mux (z, a, b, s);
  output z;
  input a, b, s;
  table
    // a  b  s : z
       0  ?  0 : 0;
       1  ?  0 : 1;
       ?  0  1 : 0;
       ?  1  1 : 1;
       0  0  x : 0;
       1  1  x : 1;
  endtable
endprimitive

module t;
  reg a, b, s;
  wire z_named, z_plain, z_d0, z_macro, z_list1, z_list2, z_str, z_str_d, z_frac;
  wire z_named_d2, z_unnamed_d2;
  int fails = 0;

  my_mux          u_named    (z_named, a, b, s);   // named, no delay (always worked)
  my_mux                     (z_plain, a, b, s);   // unnamed, no delay (always worked)
  my_mux #0                  (z_d0, a, b, s);      // unnamed after a bare delay
  my_mux `DELAY0             (z_macro, a, b, s);   // the same, from a macro
  my_mux #0                  (z_list1, a, b, s),
                             (z_list2, a, b, s);   // a list of unnamed instances
  my_mux (strong0, weak1)    (z_str, a, b, s);     // unnamed after a drive strength
  my_mux (strong0, weak1) #1 (z_str_d, a, b, s);   // strength, then delay, unnamed
  my_mux #1.5                (z_frac, a, b, s);    // fractional bare delay
  my_mux #2       u_named_d2 (z_named_d2, a, b, s);
  my_mux #2                  (z_unnamed_d2, a, b, s);

  task automatic check(string label, logic got, logic expected);
    if (got === expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %b, expected %b", label, got, expected); fails++; end
  endtask

  initial begin
    a = 1; b = 0; s = 0;                 // s = 0 selects a, so z = 1
    #10;
    check("named              z=a", z_named,  1);
    check("unnamed            z=a", z_plain,  1);
    check("unnamed #0         z=a", z_d0,     1);
    check("unnamed `macro #0  z=a", z_macro,  1);
    check("list, first        z=a", z_list1,  1);
    check("list, second       z=a", z_list2,  1);
    check("unnamed (strength) z=a", z_str,    1);
    check("unnamed (strength) #1 z=a", z_str_d, 1);
    check("unnamed #1.5       z=a", z_frac,   1);
    check("unnamed #2         z=a", z_unnamed_d2, 1);

    s = 1;                               // s = 1 selects b, so z = 0
    #10;
    check("unnamed #0         z=b", z_d0,     0);
    check("unnamed `macro #0  z=b", z_macro,  0);
    check("list, second       z=b", z_list2,  0);
    check("unnamed (strength) #1 z=b", z_str_d, 0);

    // The delay must reach the unnamed instance as it does the named one:
    // after s changes, z follows 2 time units later, not at once, not never.
    s = 0;
    #1;
    check("named   #2 not yet", z_named_d2,   0);
    check("unnamed #2 not yet", z_unnamed_d2, 0);
    #2;
    check("named   #2 arrived", z_named_d2,   1);
    check("unnamed #2 arrived", z_unnamed_d2, 1);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
