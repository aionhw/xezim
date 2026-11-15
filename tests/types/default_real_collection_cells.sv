// IEEE 1800-2023 §6.8 Table 6-7: the default of real and shortreal is 0.0.
// Each element of a class property array, a dynamic array sized by new[]
// (§7.5.1) and a nonexistent associative-array entry read (§7.8.6) takes it,
// so arithmetic on such an element is real (§11.3.1).
class C;
  real ca[2];
  shortreal cb[2][2];
  real cd[];
  real cm[int];
  function new(); cd = new[2]; endfunction
endclass
module top;
  real d[];
  shortreal sd[];
  real m[int];
  initial begin
    C c = new;
    d = new[2];
    sd = new[3];
    $display("T|class %f %f", 1/(c.ca[1]+2), 1/(c.cb[1][0]+2));
    $display("T|class_dyn %f class_assoc %f", 1/(c.cd[1]+2), 1/(c.cm[3]+2));
    $display("T|dyn %f %f assoc %f", 1/(d[1]+2), 1/(sd[2]+2), 1/(m[5]+2));
    d = new[4](d);
    $display("T|resized %f", 1/(d[3]+2));
    $finish;
  end
endmodule
