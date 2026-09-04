// Every declaration below sits at compilation-unit scope, as most DPI users
// write it; the module must see all of them.
import "DPI-C" function int us_add(int a, int b);
import "DPI-C" function void us_out_args(input int a, output int sum, output int diff);
import "DPI-C" function byte unsigned us_ub(byte unsigned b);
import "DPI-C" function shortint us_sh(shortint s);
import "DPI-C" context function int us_call_back(int v);
import "DPI-C" function chandle us_mk_handle();
import "DPI-C" function int us_rd_handle(chandle h);
import "DPI-C" function int us_logic_arg(logic l);
export "DPI-C" function us_sv_cb;
function int us_sv_cb(int v); return v * 2; endfunction

module unit_scope_dpi_test;
  int s, d;
  chandle h;
  initial begin
    us_out_args(7, s, d);
    h = us_mk_handle();
    $display("US add=%0d sum=%0d diff=%0d", us_add(2, 3), s, d);
    $display("US ub=%0d sh=%0d", us_ub(8'd255), us_sh(-5));
    $display("US cb=%0d handle=%0d", us_call_back(21), us_rd_handle(h));
    $display("US logic=%0d %0d %0d %0d", us_logic_arg(1'b1), us_logic_arg(1'b0), us_logic_arg(1'bx), us_logic_arg(1'bz));
    $finish;
  end
endmodule
