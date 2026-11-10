`timescale 1ns/1ps

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH
`define SVTEST_INIT \
  int failures = 0;
`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end
`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end
`endif

// Monitored interface: one clock, one reset, one scalar signal, one small bus.
interface UnitIfc (input clk, input rst_n);
    logic       sig;
    logic [2:0] sig_bus;
    modport MP (input clk, rst_n, input sig, sig_bus);
endinterface

// ---- DECLARATION CHURN (NOT the trigger) -------------------------------------
// Never instantiated, never referenced.  An earlier revision of this file
// claimed its mere presence was the trigger for the @()-side defect; that
// claim is WRONG and has been retracted.  It is kept only because the
// name-based fallback described in the header is sensitive to declaration
// churn, so removing it shifts which probes fail.  See README.md.
interface GhostIfc (input clk, input rst_n);
    logic ghost_sig;
    modport MP (input clk, rst_n, input ghost_sig);
endinterface;
// ------------------------------------------------------------------------------

package probe_pkg;

// Simple holder objects so a vif can be reached through a handle chain.
class HolderS;
    virtual UnitIfc.MP ifc;
endclass

class HolderA;
    virtual UnitIfc.MP ifcs[];
endclass

// ---- group 1: event-control forms on the indexed vif element ----------------

// base shape: @(posedge iarr[0].clk)
class MonPe;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(negedge iarr[0].clk)
class MonNe;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(negedge iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(iarr[0].clk) -- no edge qualifier
class MonLv;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iarr[0].sig) -- non-clock member
class MonSig;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].sig);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iarr[n-1].clk) -- expression index
class MonIdxExpr;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[n-1].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iarr[idx].clk) -- variable index
class MonIdxVar;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  idx;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            idx = 0;
            @(posedge iarr[idx].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iarr[0].clk or posedge iarr[1].clk) -- multi-edge
class MonOr;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].clk or posedge iarr[1].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iarr[0].clk iff (iarr[0].sig === 1)) -- iff qualifier
class MonIff;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].clk iff (iarr[0].sig === 1));
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(iarr[0]) -- whole interface element
class MonWhole;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
`ifndef _NO_WHOLE_VIF_IN_SENS_          
            @(iarr[0]);
`endif          
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// repeat (3) @(posedge iarr[0].clk)
class MonRep;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            repeat (3) @(posedge iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// fork @(posedge iarr[0].clk); join
class MonFj;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            fork @(posedge iarr[0].clk); join
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge ione.clk) -- scalar (non-indexed) vif property
class MonOne;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP ione;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        ione = this.iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge ione.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CONTROL: bulk assign this.iarr = iarr instead of element-wise copy
class MonBulk;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = iarr;
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// ---- group 2: wait() forms ---------------------------------------------------

// wait (iarr[2].sig === 1) -- condition never true, must stay blocked
class MonWaitSig;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (iarr[2].sig === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// wait (iarr[0].clk === 1) -- condition true at entry; a level-sensitive
// wait must return immediately, so the loop spins and trips
class MonWaitClk;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (iarr[0].clk === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// ---- group 3: receiver shapes inside @(posedge ...) --------------------------

// @(posedge this.iarr[0].clk) -- explicit this + index
class MonThis;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge this.iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge boxS.ifc.clk) -- scalar vif property of another object
class MonChainS;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    HolderS boxS;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        boxS = new;
        boxS.ifc = iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge boxS.ifc.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge boxA.ifcs[0].clk) -- indexed vif array property of another object
class MonChainA;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    HolderA boxA;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        boxA = new;
        boxA.ifcs = new[1];
        boxA.ifcs[0] = iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge boxA.ifcs[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iqueue[0].clk) -- vif held in a queue
class MonQ;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP iqueue[$];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
`ifndef _NO_VIFMPQ_      
        for (j = 0; j < n; j++) iqueue.push_back(iarr[j]);
`endif      
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
`ifndef _NO_VIFMPQ_          
            @(posedge iqueue[0].clk);
`endif          
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge imap["k0"].clk) -- vif held in a string-keyed associative array
class MonMap;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP imap[string];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        imap["k0"] = iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
`ifndef _NO_VIF_AARR_CLK_          
            @(posedge imap["k0"].clk);
`endif          
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge ifixed[1].clk) -- vif held in a fixed-size array
class MonFix;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP ifixed[3];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        for (j = 0; j < n; j++) ifixed[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge ifixed[1].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge igrid[0][1].clk) -- vif held in a 2-D array
class MonGrid;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP igrid[2][3];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        for (j = 0; j < n; j++) begin
            igrid[0][j] = iarr[j];
            igrid[1][j] = iarr[j];
        end
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge igrid[0][1].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge istat.clk) -- static vif property read bare in an instance method
class MonStatB;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    static virtual UnitIfc.MP istat;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge istat.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge MonStatQ::istatq.clk) -- class-scope-qualified static vif
class MonStatQ;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    static virtual UnitIfc.MP istatq;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge MonStatQ::istatq.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge ilocal.clk) -- task-local vif variable
class MonLocal;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        virtual UnitIfc ilocal;
`ifndef _NO_VIF_MODPORT_IN_SENS_
        ilocal = iarr[0];
`endif      
        while (!stopped) begin
`ifndef _NO_VIF_MODPORT_IN_SENS_          
            @(posedge ilocal.clk);
`endif          
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge iformal.clk) -- vif formal argument of the task
class MonFormal;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (virtual UnitIfc iformal, int max_spins);
        while (!stopped) begin
            @(posedge iformal.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// @(posedge holders[0].ifc.clk) -- vif property of a class-handle array element
class MonElem;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    HolderS holders[2];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        holders[0] = new;
        holders[1] = new;
        holders[0].ifc = iarr[0];
        holders[1].ifc = iarr[1];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge holders[0].ifc.clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// ---- group 4: wait() side, level-sensitivity probes --------------------------

// wait (ione.clk === 1) -- scalar vif; true at entry, must trip
class MonWaitOne;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    virtual UnitIfc.MP ione;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        ione = this.iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (ione.clk === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// wait (boxS.ifc.clk === 1) -- through a handle chain; true at entry, must trip
class MonWaitChS;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    HolderS boxS;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        boxS = new;
        boxS.ifc = iarr[0];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (boxS.ifc.clk === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// wait (iarr[0].sig === 1) -- tb drives sig high via NBA just after the wait
// is entered; a level-sensitive wait must wake, so the loop spins and trips
class MonWaitDrv;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (iarr[0].sig === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CONTROL: wait (counter != 0) on a plain (non-vif) class property; must trip
class MonWaitVar;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int counter;
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (counter != 0);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// ---- group 5: handle provenance and index constness -------------------------
//
// The probes above all name their vif-array property `iarr`, which is also the
// name of the interface instance array in `tb`.  That coincidence hides the
// defect: when the handle stored in the property is unusable, the simulator
// under test falls back to a *name-based* lookup of the array identifier in the
// design hierarchy, and the lookup only succeeds when the names match.
//
// Group 5 renames the property to `pifc` (no such array exists in the design)
// so the fallback cannot fire, and pairs every failing probe with a control
// that differs in exactly one thing.

// CONTROL: property named `iarr` (matches the design array) + element-wise copy
// with a NON-automatic index.  The handle is unusable, but the name fallback
// rescues it, so this probe passes on both simulators.
class MonNameHit;
    int j;
    int n;
    virtual UnitIfc.MP iarr[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc iarr[]);
        this.n = n;
        this.iarr = new[n];
        for (j = 0; j < n; j++) this.iarr[j] = iarr[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge iarr[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// Same code, property renamed to `pifc`: the name fallback cannot fire, so the
// event control never blocks in the simulator under test.
class MonNameMiss;
    int j;
    int n;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (j = 0; j < n; j++) this.pifc[j] = src[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge pifc[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CONTROL for MonNameMiss: identical except the copy loop uses an automatic
// index, which yields a usable handle, so the name fallback is not needed.
class MonNameMissAuto;
    int n;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (int k = 0; k < n; k++) this.pifc[k] = src[k];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge pifc[0].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// wait() with a VARIABLE index on a clean handle.  Element 1's `sig` is driven,
// so the wait must return and the loop must spin.
class MonWaitIdxVar;
    int n;
    int idx;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (int k = 0; k < n; k++) this.pifc[k] = src[k];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            idx = 1;
            wait (pifc[idx].sig === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// wait() with a CONSTANT index on a POISONED handle.  Element 1's `sig` is
// driven, so the wait must return and the loop must spin.
class MonWaitPoison;
    int j;
    int n;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (j = 0; j < n; j++) this.pifc[j] = src[j];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (pifc[1].sig === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CONTROL for MonWaitPoison: clean handle, same constant index.
class MonWaitClean;
    int n;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (int k = 0; k < n; k++) this.pifc[k] = src[k];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            wait (pifc[1].sig === 1);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CLEAN handle + VARIABLE index in the event control.  This is the minimal
// form of the index-constness defect: the handle is fine (automatic-index
// copy), only the index expression is not a constant.
class MonIdxVarClean;
    int n;
    int idx;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (int k = 0; k < n; k++) this.pifc[k] = src[k];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            idx = 0;
            @(posedge pifc[idx].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

// CLEAN handle + non-constant (expression) index in the event control.
class MonIdxExprClean;
    int n;
    virtual UnitIfc.MP pifc[];
    int  spins;
    bit  tripped;
    bit  stopped;

    function new (int n, virtual UnitIfc src[]);
        this.n = n;
        this.pifc = new[n];
        for (int k = 0; k < n; k++) this.pifc[k] = src[k];
        spins = 0;
    endfunction

    task run (int max_spins);
        while (!stopped) begin
            @(posedge pifc[n-1].clk);
            spins++;
            if (spins > max_spins) begin tripped = 1; stopped = 1; end
        end
    endtask
endclass

endpackage

// =============================================================================
//  Testbench: fork every monitor, idle for the watch window, then compare
//  each monitor's observed behaviour with the reference simulator's.
// =============================================================================
module tb;
    import probe_pkg::*;
    reg clk, rst_n;
    initial begin clk = 1'bx; #10 clk = 0; forever #5 clk = ~clk; end
    initial begin rst_n = 0; repeat (3) @(posedge clk); rst_n = 1; end
    UnitIfc iarr[3] (clk, rst_n);

    initial begin
        `SVTEST_INIT
        MonPe       m01;  MonNe       m02;  MonLv       m03;
        MonSig      m04;  MonIdxExpr  m05;  MonIdxVar   m06;
        MonOr       m07;  MonIff      m08;  MonWhole    m09;
        MonRep      m10;  MonFj       m11;  MonOne      m12;
        MonBulk     m13;  MonWaitSig  m14;  MonWaitClk  m15;
        MonThis     m16;  MonChainS   m17;  MonChainA   m18;
        MonQ        m19;  MonMap      m20;  MonFix      m21;
        MonGrid     m22;  MonStatB    m23;  MonStatQ    m24;
        MonLocal    m25;  MonFormal   m26;  MonElem     m27;
        MonWaitOne  m28;  MonWaitChS  m29;  MonWaitDrv  m30;
        MonWaitVar  m31;
        MonNameHit  m32;  MonNameMiss m33;  MonNameMissAuto m34;
        MonWaitIdxVar m35; MonWaitPoison m36; MonWaitClean m37;
        MonIdxVarClean m38; MonIdxExprClean m39;

        wait (rst_n === 1);

        m01 = new (3, iarr);   m02 = new (3, iarr);
        m03 = new (3, iarr);   m04 = new (3, iarr);
        m05 = new (3, iarr);   m06 = new (3, iarr);
        m07 = new (3, iarr);   m08 = new (3, iarr);
        m09 = new (3, iarr);   m10 = new (3, iarr);
        m11 = new (3, iarr);   m12 = new (3, iarr);
        m13 = new (3, iarr);   m14 = new (3, iarr);
        m15 = new (3, iarr);   m16 = new (3, iarr);
        m17 = new (3, iarr);   m18 = new (3, iarr);
        m19 = new (3, iarr);   m20 = new (3, iarr);
        m21 = new (3, iarr);   m22 = new (3, iarr);
        m23 = new (3, iarr);   m24 = new (3, iarr);
        m25 = new (3, iarr);   m26 = new (3, iarr);
        m27 = new (3, iarr);   m28 = new (3, iarr);
        m29 = new (3, iarr);   m30 = new (3, iarr);
        m31 = new (3, iarr);
        m32 = new (3, iarr);   m33 = new (3, iarr);
        m34 = new (3, iarr);   m35 = new (3, iarr);
        m36 = new (3, iarr);   m37 = new (3, iarr);
        m38 = new (3, iarr);   m39 = new (3, iarr);

        // statics and the plain-variable / driver stimuli
        MonStatB::istat  = iarr[0];
        MonStatQ::istatq = iarr[0];
        m31.counter = 1;
        iarr[0].sig <= 1'b1;
        iarr[1].sig <= 1'b1;   // watched by the group-5 wait probes

        fork
            m01.run (2000);   m02.run (2000);   m03.run (2000);
            m04.run (2000);   m05.run (2000);   m06.run (2000);
            m07.run (2000);   m08.run (2000);   m09.run (2000);
            m10.run (2000);   m11.run (2000);   m12.run (2000);
            m13.run (2000);   m14.run (2000);   m15.run (2000);
            m16.run (2000);   m17.run (2000);   m18.run (2000);
            m19.run (2000);   m20.run (2000);   m21.run (2000);
            m22.run (2000);   m23.run (2000);   m24.run (2000);
            m25.run (2000);   m26.run (iarr[0], 2000);
            m27.run (2000);   m28.run (2000);   m29.run (2000);
            m30.run (2000);   m31.run (2000);
            m32.run (2000);   m33.run (2000);   m34.run (2000);
            m35.run (2000);   m36.run (2000);   m37.run (2000);
            m38.run (2000);   m39.run (2000);
        join_none

        #2000;

        // ---- @()-side probes: the event control must block -> must NOT trip
        `SVTEST_CHECK(!m01.tripped, "MonPe       @(posedge iarr[0].clk) did not block")
        `SVTEST_CHECK(!m02.tripped, "MonNe       @(negedge iarr[0].clk) did not block")
        `SVTEST_CHECK(!m03.tripped, "MonLv       @(iarr[0].clk) did not block")
        `SVTEST_CHECK(!m04.tripped, "MonSig      @(posedge iarr[0].sig) did not block")
        `SVTEST_CHECK(!m05.tripped, "MonIdxExpr  @(posedge iarr[n-1].clk) did not block")
        `SVTEST_CHECK(!m06.tripped, "MonIdxVar   @(posedge iarr[idx].clk) did not block")
        `SVTEST_CHECK(!m07.tripped, "MonOr       @(posedge iarr[0].clk or ...) did not block")
        `SVTEST_CHECK(!m08.tripped, "MonIff      @(posedge iarr[0].clk iff ...) did not block")
        `SVTEST_CHECK(!m09.tripped, "MonWhole    @(iarr[0]) did not block")
        `SVTEST_CHECK(!m10.tripped, "MonRep      repeat(3) @(posedge iarr[0].clk) did not block")
        `SVTEST_CHECK(!m11.tripped, "MonFj       fork @(posedge iarr[0].clk); join did not block")
        `SVTEST_CHECK(!m12.tripped, "MonOne      @(posedge ione.clk) did not block")
        `SVTEST_CHECK(!m13.tripped, "MonBulk     bulk-assign control did not block")
        `SVTEST_CHECK(!m14.tripped, "MonWaitSig  wait (iarr[2].sig === 1) must stay blocked")
        `SVTEST_CHECK(!m16.tripped, "MonThis     @(posedge this.iarr[0].clk) did not block")
        `SVTEST_CHECK(!m17.tripped, "MonChainS   @(posedge boxS.ifc.clk) did not block")
        `SVTEST_CHECK(!m18.tripped, "MonChainA   @(posedge boxA.ifcs[0].clk) did not block")
        `SVTEST_CHECK(!m19.tripped, "MonQ        @(posedge iqueue[0].clk) did not block")
        `SVTEST_CHECK(!m20.tripped, "MonMap      @(posedge imap[k0].clk) did not block")
        `SVTEST_CHECK(!m21.tripped, "MonFix      @(posedge ifixed[1].clk) did not block")
        `SVTEST_CHECK(!m22.tripped, "MonGrid     @(posedge igrid[0][1].clk) did not block")
        `SVTEST_CHECK(!m23.tripped, "MonStatB    @(posedge istat.clk) did not block")
        `SVTEST_CHECK(!m24.tripped, "MonStatQ    @(posedge MonStatQ::istatq.clk) did not block")
        `SVTEST_CHECK(!m25.tripped, "MonLocal    @(posedge ilocal.clk) did not block")
        `SVTEST_CHECK(!m26.tripped, "MonFormal   @(posedge iformal.clk) did not block")
        `SVTEST_CHECK(!m27.tripped, "MonElem     @(posedge holders[0].ifc.clk) did not block")

        // ---- wait()-side probes: level-sensitive wait must return -> must trip
        `SVTEST_CHECK(m15.tripped, "MonWaitClk  wait (iarr[0].clk === 1) must be level-sensitive")
        `SVTEST_CHECK(m28.tripped, "MonWaitOne  wait (ione.clk === 1) must be level-sensitive")
        `SVTEST_CHECK(m29.tripped, "MonWaitChS  wait (boxS.ifc.clk === 1) must be level-sensitive")
        `SVTEST_CHECK(m30.tripped, "MonWaitDrv  wait (iarr[0].sig === 1) must wake on the NBA")
        `SVTEST_CHECK(m31.tripped, "MonWaitVar  wait (counter != 0) must be level-sensitive")

        // ---- group 5: handle provenance and index constness -----------------
        // @()-side: must block -> must NOT trip
        `SVTEST_CHECK(!m32.tripped, "MonNameHit  @(posedge iarr[0].clk) name fallback did not block")
        `SVTEST_CHECK(!m33.tripped, "MonNameMiss @(posedge pifc[0].clk) poisoned handle did not block")
        `SVTEST_CHECK(!m34.tripped, "MonNameMissAuto @(posedge pifc[0].clk) clean handle did not block")
        // wait()-side: element 1's sig is driven -> must trip
        `SVTEST_CHECK(m35.tripped, "MonWaitIdxVar wait (pifc[idx].sig === 1) must wake on a variable index")
        `SVTEST_CHECK(m36.tripped, "MonWaitPoison wait (pifc[1].sig === 1) must wake on a poisoned handle")
        `SVTEST_CHECK(m37.tripped, "MonWaitClean  wait (pifc[1].sig === 1) must wake on a clean handle")
        `SVTEST_CHECK(!m38.tripped, "MonIdxVarClean  @(posedge pifc[idx].clk) did not block")
        `SVTEST_CHECK(!m39.tripped, "MonIdxExprClean @(posedge pifc[n-1].clk) did not block")

        `SVTEST_PASSFAIL
        $finish;
    end
endmodule
