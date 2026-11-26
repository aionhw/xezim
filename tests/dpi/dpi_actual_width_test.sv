// The actual of a DPI import's input formal is assigned to the formal on
// entry (IEEE 1800-2023 13.5, 35.5.6), so it is evaluated in the formal's
// width, like the right-hand side of an assignment (11.6.1, 11.8.1): the
// operands of `|`, `+`, `-` and the left operand of `<<` are extended to
// the formal's width before the operation. C returns what it received.
typedef int        my_int_t;
typedef bit [15:0] u16_t;
import "DPI-C" function int      dpi_echo_int(input int v);
import "DPI-C" function shortint dpi_echo_short(input shortint v);
import "DPI-C" function longint  dpi_echo_long(input longint v);
import "DPI-C" function int      dpi_echo_td(input my_int_t v);
import "DPI-C" function int      dpi_echo_u16(input u16_t v);
import "DPI-C" function int      dpi_echo_b16(input bit [15:0] v);
import "DPI-C" function int      dpi_echo_l16(input logic [15:0] v);
import "DPI-C" function int      dpi_echo_dflt(input int v = 8'd200 + 8'd100);
import "DPI-C" function int      dpi_add2(input int a, b);
import "DPI-C" function bit      dpi_echo_bit(input bit v);
import "DPI-C" function void     dpi_inc(inout int v);
import "DPI-C" task              dpi_task_echo(input int v, output int o);

function automatic void check_int(string what, longint got, longint expected);
  if (got == expected) $display("PASS %s = %0d", what, got);
  else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
endfunction

class flags_t;
  bit flag_a;
  bit flag_b;
  byte unsigned x;
  byte unsigned y;
  function new();
    flag_a = 0;  flag_b = 1;  x = 200;  y = 100;
  endfunction
  task in_method();
    int o;
    check_int("method: a | (b << 1) over bit members", dpi_echo_int(flag_a | (flag_b << 1)), 2);
    check_int("method: x + y over 8-bit members",      dpi_echo_int(x + y), 300);
    check_int("method: named actual",                  dpi_echo_int(.v(x + y)), 300);
    check_int("method: bit [15:0] formal",             dpi_echo_b16(x + y), 300);
    check_int("method: typedef int formal",            dpi_echo_td(x + y), 300);
    dpi_task_echo(x + y, o);
    check_int("method: task input",                    o, 300);
  endtask
endclass

module top;
  bit  a, b;
  byte unsigned x, y;
  int  one, io, o;
  initial begin
    flags_t f = new;
    a = 0;  b = 1;  x = 200;  y = 100;  one = 1;
    check_int("a | (b << 1) over bit operands", dpi_echo_int(a | (b << 1)), 2);
    check_int("x + y over 8-bit operands",      dpi_echo_int(x + y), 300);
    check_int("shortint formal, x + y",         dpi_echo_short(x + y), 300);
    check_int("longint formal, int << 40",      dpi_echo_long(one << 40), 64'd1 << 40);
    check_int("through a class handle",         dpi_echo_int(f.flag_a | (f.flag_b << 1)), 2);
    check_int("signed formal, x - y - 150",     dpi_echo_int(x - y - 8'd150), -50);
    check_int("typedef int formal",             dpi_echo_td(x + y), 300);
    check_int("typedef bit [15:0] formal",      dpi_echo_u16(x + y), 300);
    check_int("bit [15:0] formal",              dpi_echo_b16(x + y), 300);
    check_int("logic [15:0] formal",            dpi_echo_l16(x + y), 300);
    check_int("default value",                  dpi_echo_dflt(), 300);
    check_int("inherited formal type",          dpi_add2(x + y, x + y), 600);
    check_int("inherited formal type, values",  dpi_add2(3, 5), 8);
    check_int("named actual",                   dpi_echo_int(.v(a | (b << 1))), 2);
    dpi_task_echo(x + y, o);
    check_int("task input",                     o, 300);
    // unchanged: a one-bit formal, a self-determined result, an inout
    check_int("bit formal",                     dpi_echo_bit(a | (b << 1)), 0);
    check_int("comparison stays one bit",       dpi_echo_int(a == b), 0);
    check_int("concatenation keeps its width",  dpi_echo_int({b, a}), 2);
    check_int("a value that fits",              dpi_echo_int(x), 200);
    check_int("real actual",                    dpi_echo_int(3.0 + 4.0), 7);
    io = 5; dpi_inc(io);
    check_int("inout",                          io, 6);
    f.in_method();
    $finish;
  end
endmodule
