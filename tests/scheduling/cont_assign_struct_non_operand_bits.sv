`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2: unpacked struct arrays.
// §13.4/§13.5.1: automatic input functions, no time controls.
// §10.3.2: unrelated writes are not operands; §20.3.1 observes evaluation time.
// §20.6.1/§20.6.2/§20.7: type and fixed-array shape queries do not read data.
// §3.12.1: $unit qualification selects the compilation-unit declaration.
// §4.7/§6.8: one write per positive time slot, check a unit later.
typedef struct { int v; int tag; } S;
module top;
  S arr[2];

  function automatic longint stamp(input int unused); return $time; endfunction
  longint result;
  assign result = stamp($bits(arr));
  initial begin
    #1 arr[0].v = 1;
    #1 if (result === 0) $display("PASS first");
       else $display("FAIL first got=%0d expected=0", result);
    #1 arr[0].v = 3;
    #1 if (result === 0) $display("PASS second");
       else $display("FAIL second got=%0d expected=0", result);
    $finish;
  end
endmodule
