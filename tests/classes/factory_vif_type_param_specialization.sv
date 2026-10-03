interface I ();
  event e;
  int cnt = 0;
  function void fire(); -> e; endfunction
endinterface

package uvc;
  class reg_#(type T=int);
    static function T create(string n);
      T t = new(n);
      return t;
    endfunction
  endclass
  class base;
    string name;
    function new(string n=""); name=n; endfunction
  endclass
  class drv#(type T=int) extends base;
    T vif;
    `ifndef NO_REG
    typedef reg_#(drv#(T)) type_id;
    `endif
    static function drv#(T) create2(string n); drv#(T) d=new(n); return d; endfunction
    task run();
      #3;
      vif.fire();          // must fire the interface's event
    endtask
  endclass
  class env#(type T=int) extends base;
    drv#(T) d;
    typedef reg_#(env#(T)) type_id;
    function void build();
      d = drv#(T)::type_id::create("d");
    endfunction
  endclass
endpackage

module top;
  I pif();
  uvc::env#(virtual I) e;
  int hits = 0;
  initial forever begin @(pif.e); hits++; end
  initial begin
    e = uvc::env#(virtual I)::type_id::create("e");
    e.build();
    e.d.vif = top.pif;      // nested-handle LHS, interface instance value
    fork e.d.run(); join_none
    #7;
    if (hits == 1 && e.d.vif == top.pif) $display("TAG_PASS hits=%0d", hits);
    else $display("TAG_FAIL hits=%0d", hits);
  end
endmodule
