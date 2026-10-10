// Regression for a process-context clobber when a NO_MAP register's SECOND
// backdoor `write` trampolines its resumed `do_write` continuation (with an
// inlined `foreach (m_fields[i])` that must resolve `this`) through the event
// queue. Before the fix, `run_scheduled_process_inner` treated the resumed
// parent like a fork child: it `take_process_context()`'d the live `this` and
// restored an EMPTY table entry, so the field loop's `this` resolved to
// module/phantom and null-deref'd on a 25+-field register. The second write
// is what pushes the inlined-task chain past the trampoline depth, so the
// bug only appears on `read + peek + write + write`.
//
// The observable is the `[xezim][error] null object dereference:
// 'map_info.frontdoor'` line xezim logs (internally caught) when the field
// loop reads through the phantom null `this`. The harness's `assert_summary`
// rejects any `[xezim][error]`/`[xezim][fatal]`, so this bench fails on the
// unfixed simulator and passes once the fix is in. It is 1800.2-2020 only:
// UVM 1.2's `UVM_NO_DPI` build has backdoor uvm_hdl compiled off and aborts
// the access with a UVM_FATAL before reaching the register loop.

import uvm_pkg::*;
`include "uvm_macros.svh"

program top;
  class r1_typ extends uvm_reg;
    uvm_reg_field WO1; uvm_reg_field W1; uvm_reg_field WOS; uvm_reg_field WOC; uvm_reg_field WO;
    uvm_reg_field W0CRS; uvm_reg_field W0SRC; uvm_reg_field W1CRS; uvm_reg_field W1SRC; uvm_reg_field W0T;
    uvm_reg_field W0S; uvm_reg_field W0C; uvm_reg_field W1T; uvm_reg_field W1S; uvm_reg_field W1C;
    uvm_reg_field WCRS; uvm_reg_field WSRC; uvm_reg_field WS; uvm_reg_field WC; uvm_reg_field WRS;
    uvm_reg_field WRC; uvm_reg_field RS; uvm_reg_field RC; uvm_reg_field RW; uvm_reg_field RO;
    function new(string name = "a_reg"); super.new(name,64,UVM_NO_COVERAGE); endfunction
    virtual function void build();
      this.WO1   = new("WO1");   this.W1=new("W1");   this.WOS=new("WOS"); this.WOC=new("WOC"); this.WO=new("WO");
      this.W0CRS=new("W0CRS"); this.W0SRC=new("W0SRC"); this.W1CRS=new("W1CRS"); this.W1SRC=new("W1SRC"); this.W0T=new("W0T");
      this.W0S=new("W0S"); this.W0C=new("W0C"); this.W1T=new("W1T"); this.W1S=new("W1S"); this.W1C=new("W1C");
      this.WCRS=new("WCRS"); this.WSRC=new("WSRC"); this.WS=new("WS"); this.WC=new("WC"); this.WRS=new("WRS");
      this.WRC=new("WRC"); this.RS=new("RS"); this.RC=new("RC"); this.RW=new("RW"); this.RO=new("RO");
      this.WO1.configure(this, 2, 48,"WO1", 0,2'b01,1,0,0); this.W1.configure(this,2,46,"W1",0,2'b01,1,0,0);
      this.WOS.configure(this,2,44,"WOS",0,2'b01,1,0,0); this.WOC.configure(this,2,42,"WOC",0,2'b01,1,0,0); this.WO.configure(this,2,40,"WO",0,2'b01,1,0,0);
      this.W0CRS.configure(this,2,38,"W0CRS",0,2'b01,1,0,0); this.W0SRC.configure(this,2,36,"W0SRC",0,2'b01,1,0,0);
      this.W1CRS.configure(this,2,34,"W1CRS",0,2'b01,1,0,0); this.W1SRC.configure(this,2,32,"W1SRC",0,2'b01,1,0,0);
      this.W0T.configure(this,2,30,"W0T",0,2'b01,1,0,0); this.W0S.configure(this,2,28,"W0S",0,2'b01,1,0,0); this.W0C.configure(this,2,26,"W0C",0,2'b01,1,0,0);
      this.W1T.configure(this,2,24,"W1T",0,2'b01,1,0,0); this.W1S.configure(this,2,22,"W1S",0,2'b01,1,0,0); this.W1C.configure(this,2,20,"W1C",0,2'b01,1,0,0);
      this.WCRS.configure(this,2,18,"WCRS",0,2'b01,1,0,0); this.WSRC.configure(this,2,16,"WSRC",0,2'b01,1,0,0);
      this.WS.configure(this,2,14,"WS",0,2'b01,1,0,0); this.WC.configure(this,2,12,"WC",0,2'b01,1,0,0);
      this.WRS.configure(this,2,10,"WRS",0,2'b01,1,0,0); this.WRC.configure(this,2,8,"WRC",0,2'b01,1,0,0);
      this.RS.configure(this,2,6,"RS",0,2'b01,1,0,0); this.RC.configure(this,2,4,"RC",0,2'b01,1,0,0);
      this.RW.configure(this,2,2,"RW",0,2'b01,1,0,0); this.RO.configure(this,2,0,"RO",0,2'b01,1,0,0);
    endfunction
    `uvm_object_utils(r1_typ)
  endclass

  class b1_typ extends uvm_reg_block;
    rand r1_typ r1;
    function new(string name = "b1_typ"); super.new(name,UVM_NO_COVERAGE); endfunction
    virtual function void build();
      r1 = r1_typ::type_id::create("r1");
      r1.build();
      r1.configure(this,null,"r1");
    endfunction
    `uvm_object_utils(b1_typ)
  endclass

  initial begin
    b1_typ model;
    uvm_status_e st;
    uvm_reg_data_t d;
    model = new("model");
    model.build();
    model.set_hdl_path_root("$root.top");
    model.reset();

    // ---- backdoor read + peek (part of the crash trigger: the resumed
    // `do_write`/context chain must first be pushed deep enough that the
    // SECOND `write` trampolines its inlined field loop past the recursion
    // depth; a `read`+`peek` before the writes is what deepens that chain) ----
    model.r1.read(st, d, .path(UVM_BACKDOOR));
    model.r1.peek(st, d);

    // ---- first backdoor write ----
    model.r1.write(st, 64'h2AAAAAAAAAAAA, .path(UVM_BACKDOOR));
    model.r1.peek(st, d);
    $display("T|write1|mirror=%h", model.r1.get());

    // ---- second backdoor write: must run the field loop with `this` intact ----
    model.r1.write(st, 64'h16AAAAAAAAAAA, .path(UVM_BACKDOOR));
    model.r1.peek(st, d);
    $display("T|write2|mirror=%h", model.r1.get());

    $display("TEST_DONE");
  end
endprogram