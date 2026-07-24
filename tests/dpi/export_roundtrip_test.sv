// §35.5.4 DPI export: C (imported context functions) calls back into exported
// SystemVerilog functions and tasks across int / longint / real / string / void.
module tb;
  export "DPI-C" function sv_scale;
  export "DPI-C" function sv_combine;
  export "DPI-C" task     sv_record;
  export "DPI-C" function sv_widen;
  export "DPI-C" function sv_half;
  export "DPI-C" task     sv_note;

  int      recorded = -1;
  longint  widened  = 0;
  string   noted    = "";

  function int      sv_scale(int x);            return x * 3;                  endfunction
  function int      sv_combine(int a, int b);   return a + b;                  endfunction
  task              sv_record(int v);           recorded = v;                  endtask
  function longint  sv_widen(longint x);        return x + 64'h1_0000_0000;    endfunction
  function real     sv_half(real x);            return x / 2.0;                endfunction
  task              sv_note(string s);          noted = s;                     endtask

  import "DPI-C" context function int      c_roundtrip(input int x);
  import "DPI-C" context function longint  c_widen(input longint v);
  import "DPI-C" context function int      c_real_and_str();

  int failures = 0;
  int r; longint w;
  initial begin
    r = c_roundtrip(10);          // 30; 40; record 40; return 41
    if (r != 41)               begin failures++; $display("FAIL: return %0d != 41", r); end
    if (recorded != 40)        begin failures++; $display("FAIL: recorded %0d != 40", recorded); end

    w = c_widen(64'h5);          // 5 + 2^32
    if (w != 64'h1_0000_0005)  begin failures++; $display("FAIL: widened %h", w); end

    r = c_real_and_str();        // sv_half(9.0)=4.5 -> *10 = 45; sv_note("world")
    if (r != 45)               begin failures++; $display("FAIL: real*10 %0d != 45", r); end
    if (noted != "world")      begin failures++; $display("FAIL: noted '%s' != world", noted); end

    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    $finish;
  end
endmodule
