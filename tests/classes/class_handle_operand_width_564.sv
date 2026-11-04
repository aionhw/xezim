typedef struct packed { bit [7:0] sa, sb; } st_t;

class E;
  bit [7:0] arr [4];             // check 2
  st_t      st;                  // check 1

  function new();
    arr = '{200, 100, 200, 100};
    st.sa = 200; st.sb = 100;
  endfunction
endclass

module top;
  int fails;
  logic [8:0] s;
  E e;

  function automatic void mchk(int r, bit g, string nm, ref int n);
    if (!g) begin n++; $display("  FAIL %-4d %s", r, nm); end
    else    begin      $display("  ok   %-4d %s", r, nm); end
  endfunction

  initial begin
    fails = 0;
    s = 300;
    e = new;

    mchk(1, s === e.st.sa + e.st.sb,   "e.st.sa + e.st.sb",     fails);
    mchk(2, s === e.arr[0] + e.arr[1], "e.arr[0] + e.arr[1]",   fails);

    // -- controls: these already passed before the fix, and pin its scope
    begin
      logic [8:0] t9;
      t9 = e.st.sa + e.st.sb;
      mchk(3, t9 == 300, "control: assignment context already worked", fails);
    end
    
    if (fails == 0) $display("TEST_PASS");
    else            $display("TEST_FAIL count=%0d", fails);
    $finish;
  end
endmodule
