// Reporter's consolidated reproducer for #925 (unchanged): virtual-interface
// collections as constructor/task/function formals, as task locals and in
// 2-D arrays; event controls and waits through them. Groups B/C/F are the
// regression guard. Self-checking: FAIL lines + TEST_PASS.
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

// Guarded event probe: records FIRED(1)/STUCK(2)/SPIN(3) into res__.
// The #1 pre-step + odd watchdog keep probe starts OFF the clock grid (a
// start exactly on a posedge lets a correct sim catch the same-step edge).
`define VPROBE(res__, evt) \
  begin \
    time t0__; bit fired__; \
    #1; \
    t0__ = $time; fired__ = 1'b0; \
    fork \
      begin @(evt); fired__ = ($time > t0__); end \
      begin #203; end \
    join_any \
    disable fork; \
    if (fired__)           res__ = 1; \
    else if ($time > t0__) res__ = 2; \
    else                   res__ = 3; \
  end

`define VWAIT(res__, cond) \
  begin \
    time t0__; bit done__; \
    t0__ = $time; done__ = 1'b0; \
    fork \
      begin wait (cond); done__ = 1'b1; end \
      begin #200; end \
    join_any \
    disable fork; \
    if (done__)            res__ = 1; \
    else if ($time > t0__) res__ = 2; \
    else                   res__ = 3; \
  end

// Report + check one recorded outcome against its expectation.
`define REPORT(tag, got__, exp__) \
  begin \
    $display("%-32s obs=%0d exp=%0d t=%0t", tag, got__, exp__, $time); \
    `SVTEST_CHECK((got__) == (exp__), tag) \
  end

interface InputInterface (input clk, input rst_l);
    logic        next_in;
    logic [2:0]  in_valid;
    bit          go = 1;

    clocking cb @(posedge clk);
        input rst_l, go;
    endclocking

    modport MonitoringInterface(input clk, rst_l,
                        input next_in,
                        input in_valid);
endinterface
// ---------------------------------------------------------------------------
// Group A: the headline shape (original MWE, unchanged).
// Property `inputIntf` vs module array `intf` -- DIFFERENT names; matching
// names mask the bug through module-scope lookup (name coincidence, not fix).
// ---------------------------------------------------------------------------
class HangMonitor;
    int  cycleCount;
    bit  formal_bound;      // A1 [BUG]: ctor formal element != null
    bit  member_read_ok;    // A2 [BUG]: property element member read
    int  evt;               // A3 [BUG]: @(posedge inputIntf[0].clk)
    virtual InputInterface.MonitoringInterface inputIntf[];

    function new(int numInputs, virtual InputInterface inputIntf[]);
        this.inputIntf = new[numInputs];
        for (int i = 0; i < numInputs; i++)
            this.inputIntf[i] = inputIntf[i];
        formal_bound = (inputIntf[0] != null);
        cycleCount = 0;
    endfunction

    task run();
        member_read_ok = (inputIntf[0].rst_l === 1'b1);
        `VPROBE(evt, posedge inputIntf[0].clk)
        if (evt == 1) cycleCount++;
    endtask
endclass

// ---------------------------------------------------------------------------
// Group B: event controls on vif COLLECTION ELEMENTS from class-method scope
// (bare name = implicit `this`, the failing context).  Discriminator: the
// [0] elements carry a RUNNING clock, the [1] elements a STOPPED one -- so a
// wrong-element resolution cannot hide (it would show FIRED where STUCK is
// expected).  These all PASS in xezim: regression guard for the fix.
// ---------------------------------------------------------------------------
class Elem;
    int e_dyn0, e_dyn1, e_this, e_fix0, e_fix1, e_que0, e_asc3, e_scal,
        e_wait, e_iff, e_or, e_cb;
    bit r_dyn, r_fix, r_que, r_asc;
    virtual InputInterface vd[];
    virtual InputInterface vf[2];
    virtual InputInterface vq[$];
    virtual InputInterface va[int];
    virtual InputInterface vs;          // scalar control

    task automatic m_dyn0(); `VPROBE(e_dyn0, posedge vd[0].clk)  endtask
    task automatic m_dyn1(); `VPROBE(e_dyn1, posedge vd[1].clk)  endtask
    task automatic m_this(); `VPROBE(e_this, posedge this.vd[0].clk) endtask
    task automatic m_fix0(); `VPROBE(e_fix0, posedge vf[0].clk)  endtask
    task automatic m_fix1(); `VPROBE(e_fix1, posedge vf[1].clk)  endtask
    task automatic m_que0(); `VPROBE(e_que0, posedge vq[0].clk)  endtask
    task automatic m_asc3(); `VPROBE(e_asc3, posedge va[3].clk)  endtask
    task automatic m_scal(); `VPROBE(e_scal, posedge vs.clk)     endtask
    task automatic m_wait(); `VWAIT(e_wait, vd[0].go)            endtask
    task automatic m_iff();  `VPROBE(e_iff, posedge vd[0].clk iff vd[0].rst_l) endtask
    task automatic m_or();   `VPROBE(e_or,  posedge vd[0].clk or negedge vd[0].rst_l) endtask
    task automatic m_cb();   `VPROBE(e_cb,  vf[0].cb)            endtask

    task automatic reads();
        r_dyn = (vd[0] != null) && (vd[0].rst_l === 1'b1);
        r_fix = (vf[0] != null) && (vf[0].rst_l === 1'b1);
        r_que = (vq[0] != null) && (vq[0].rst_l === 1'b1);
        r_asc = (va[3] != null) && (va[3].rst_l === 1'b1);
    endtask
endclass

// nested handle chain: e.vd[0].clk resolved from ANOTHER class's method
class Nest;
    int  e_nest;
    Elem e;
    task automatic m_nest(); `VPROBE(e_nest, posedge e.vd[0].clk) endtask
endclass
// ---------------------------------------------------------------------------
// Group D: vif formals.  arguments.rs has dedicated binders for assoc /
// collection / struct formals and for plain-scalar formals, but a vif-typed
// DYNAMIC-ARRAY / QUEUE formal falls through -> elements read null [BUG].
// The assoc formal (D2) is the tell: it HAS a binder, so it works.
// ---------------------------------------------------------------------------
class Formals;
    bit b_que, b_asc, b_dyn, b_prop, b_task, b_func, b_scal;
    int evt_formal;
    virtual InputInterface tq[1], ta[1], td[1], tprop[1], ttask[1], tfunc[1];

    function automatic void f_que(virtual InputInterface f[$]);
        tq[0] = f[0];
    endfunction
    function automatic void f_asc(virtual InputInterface f[int]);
        ta[0] = f[3];
    endfunction
    function automatic void f_dyn(virtual InputInterface f[]);
        td[0] = f[0];
    endfunction
    function automatic void f_prop(virtual InputInterface f[]);
        tprop[0] = f[0];
    endfunction
    task automatic t_arr(virtual InputInterface f[]);
        ttask[0] = f[0];
    endtask
    function automatic void fn_arr(virtual InputInterface f[]);
        tfunc[0] = f[0];
    endfunction
    task automatic t_evtformal(virtual InputInterface f[]);
        `VPROBE(evt_formal, posedge f[0].clk)
    endtask
    function automatic void f_scalar(virtual InputInterface v);
        b_scal = (v != null) && (v.rst_l === 1'b1);
    endfunction
    task automatic reads();
        b_que  = (tq[0]    != null) && (tq[0].rst_l    === 1'b1);
        b_asc  = (ta[0]    != null) && (ta[0].rst_l    === 1'b1);
        b_dyn  = (td[0]    != null) && (td[0].rst_l    === 1'b1);
        b_prop = (tprop[0] != null) && (tprop[0].rst_l === 1'b1);
        b_task = (ttask[0] != null) && (ttask[0].rst_l === 1'b1);
        b_func = (tfunc[0] != null) && (tfunc[0].rst_l === 1'b1);
    endtask
endclass

// ---------------------------------------------------------------------------
// Group E: vif collection STORAGE shapes (2D, string-key assoc, static).
// ---------------------------------------------------------------------------
class Store;
    bit b_2d, b_str, b_static;
    int e_2d;
    virtual InputInterface v2[2][2];
    virtual InputInterface vs[string];
    static virtual InputInterface sd[];

    task automatic reads();
        b_2d     = (v2[0][1] != null) && (v2[0][1].rst_l === 1'b1);
        b_str    = (vs["k"]  != null) && (vs["k"].rst_l  === 1'b1);
        b_static = (sd[0]    != null) && (sd[0].rst_l    === 1'b1);
    endtask
    task automatic m_2d(); `VPROBE(e_2d, posedge v2[0][1].clk) endtask
endclass
// ---------------------------------------------------------------------------
// Testbench.  Group E also covers task- / function-LOCAL vif arrays:
// function-local works (control E8), task-local does not [BUG] -> the loss
// is in the TASK frame, not in vif-array storage.
// ---------------------------------------------------------------------------
module testbench;
    // outcome codes: 1=FIRED 2=STUCK 3=SPIN (see header)
    bit clkA = 1'b0;              // RUNNING
    bit clkB = 1'b0;              // STOPPED forever
    bit rst_l = 1'b1;
    always #5 clkA = ~clkA;

    localparam int W = 2;

    // headline pair: module array named `intf`, class property `inputIntf`
    InputInterface intf[2]  (clkA, rst_l);
    InputInterface runA     (clkA, rst_l);
    InputInterface runB     (clkA, rst_l);
    InputInterface deadA    (clkB, rst_l);
    InputInterface deadB    (clkB, rst_l);
    InputInterface iarr[2]  (clkA, rst_l);   // interface INSTANCE ARRAY

    HangMonitor hm;
    Elem    o;
    Nest    nn;
    Store   st;
    Formals fm;

    virtual InputInterface varr[];            // module-scope vif ARRAY VAR
    virtual InputInterface vsvar;             // module-scope scalar control
    virtual InputInterface q[$];
    virtual InputInterface a[int];

    bit b_lfix, b_ldyn, b_lfunc;
    int e_ldyn, e_arrvar, e_scalvar, e_instarr, e_paramidx;
    int n_foreach;

    task automatic local_fix(virtual InputInterface inst);
        virtual InputInterface lv[1];
        lv[0] = inst;
        b_lfix = (lv[0] != null) && (lv[0].rst_l === 1'b1);
    endtask

    task automatic local_dyn(virtual InputInterface inst);
        virtual InputInterface ld[];
        ld = new[1];
        ld[0] = inst;
        b_ldyn = (ld[0] != null) && (ld[0].rst_l === 1'b1);
        `VPROBE(e_ldyn, posedge ld[0].clk)
    endtask

    function automatic bit local_func(virtual InputInterface inst);
        virtual InputInterface lf[];
        lf = new[1];
        lf[0] = inst;
        return (lf[0] != null) && (lf[0].rst_l === 1'b1);
    endfunction

    task automatic count_foreach();
        n_foreach = 0;
        foreach (o.vd[i]) n_foreach++;
    endtask

    initial begin
        `SVTEST_INIT

        // ---- bind everything from real instances ----
        o = new; nn = new; nn.e = o; st = new; fm = new;
        o.vd = new[2];
        o.vd[0] = runA;  o.vd[1] = deadA;          // [0] runs, [1] dead
        o.vf[0] = runA;  o.vf[1] = deadB;
        o.vq.push_back(runA);
        o.vq.push_back(deadA);
        o.va[3] = runA;  o.va[7] = deadB;
        o.vs    = runA;
        st.v2[0][1] = runA;
        st.vs["k"]  = runA;
        Store::sd    = new[1];
        Store::sd[0] = runA;
        q.push_back(runA);
        a[3] = runA;
        varr = new[1];
        varr[0] = runA;
        vsvar = runA;
        #1;

        // ---- group A: headline (original MWE) ----
        hm = new(2, intf);
        hm.run();
        `SVTEST_CHECK(hm.formal_bound,   "A1_ctor_vif_array_formal_bound")
        `SVTEST_CHECK(hm.member_read_ok, "A2_dyn_property_elem_member_read")
        `REPORT("A3_headline_dyn_elem_evt", hm.evt, 1)

        // ---- group B: property-element event controls, class scope ----
        o.reads();
        `SVTEST_CHECK(o.r_dyn, "B0_read_dyn_elem")
        `SVTEST_CHECK(o.r_fix, "B0_read_fix_elem")
        `SVTEST_CHECK(o.r_que, "B0_read_que_elem")
        `SVTEST_CHECK(o.r_asc, "B0_read_asc_elem")
        o.m_dyn0();  `REPORT("B1_dyn0_running",     o.e_dyn0,  1)
        o.m_dyn1();  `REPORT("B2_dyn1_stopped",     o.e_dyn1,  2)
        o.m_this();  `REPORT("B3_this_dyn0",        o.e_this,  1)
        o.m_fix0();  `REPORT("B4_fix0_running",     o.e_fix0,  1)
        o.m_fix1();  `REPORT("B5_fix1_stopped",     o.e_fix1,  2)
        o.m_que0();  `REPORT("B6_que0_running",     o.e_que0,  1)
        o.m_asc3();  `REPORT("B7_asc3_running",     o.e_asc3,  1)
        o.m_scal();  `REPORT("B8_scalar_control",   o.e_scal,  1)
        o.m_wait();  `REPORT("B9_wait_on_elem_bit", o.e_wait,  1)
        o.m_iff();   `REPORT("B10_iff_clause",      o.e_iff,   1)
        o.m_or();    `REPORT("B11_or_list",         o.e_or,    1)
        o.m_cb();    `REPORT("B12_cb_on_fix_elem",  o.e_cb,    1)
        nn.m_nest(); `REPORT("B13_nested_handle",   nn.e_nest, 1)

        // ---- group C: module scope, same shapes ----
        `VPROBE(e_arrvar, posedge varr[0].clk)
        `REPORT("C1_module_vif_array_var", e_arrvar,  1)
        `VPROBE(e_scalvar, posedge vsvar.clk)
        `REPORT("C2_module_scalar_vif",    e_scalvar, 1)

        // ---- group D: vif formals ----
        fm.f_que(q);
        fm.f_asc(a);
        fm.f_dyn(varr);
        fm.f_prop(o.vd);
        fm.t_arr(varr);
        fm.fn_arr(varr);
        fm.reads();
        `SVTEST_CHECK(fm.b_que,  "D1_queue_formal_bound")
        `SVTEST_CHECK(fm.b_asc,  "D2_assoc_formal_bound")
        `SVTEST_CHECK(fm.b_dyn,  "D3_dyn_array_formal_bound")
        `SVTEST_CHECK(fm.b_prop, "D4_formal_actual_is_class_property")
        `SVTEST_CHECK(fm.b_task, "D5_task_array_formal_bound")
        `SVTEST_CHECK(fm.b_func, "D6_func_array_formal_bound")
        fm.t_evtformal(varr);
        `REPORT("D7_evt_on_formal_element", fm.evt_formal, 1)
        fm.f_scalar(o.vf[0]);
        `SVTEST_CHECK(fm.b_scal, "D8_scalar_formal_elem_actual")

        // ---- group E: storage shapes ----
        st.reads();
        `SVTEST_CHECK(st.b_2d,     "E1_2d_vif_array_read")
        `SVTEST_CHECK(st.b_str,    "E2_string_key_assoc_read")
        `SVTEST_CHECK(st.b_static, "E3_static_vif_array_read")
        st.m_2d(); `REPORT("E4_2d_vif_array_evt", st.e_2d, 1)
        local_fix(runA);
        `SVTEST_CHECK(b_lfix, "E5_task_local_fixed_array")
        local_dyn(runA);
        `SVTEST_CHECK(b_ldyn, "E6_task_local_dyn_array")
        `REPORT("E7_task_local_dyn_evt", e_ldyn, 1)
        b_lfunc = local_func(runA);
        `SVTEST_CHECK(b_lfunc, "E8_function_local_dyn_array")
        count_foreach();
        `SVTEST_CHECK(n_foreach == 2, "E9_foreach_over_dyn_vif")

        // ---- group F: controls ----
        `VPROBE(e_instarr, posedge iarr[0].clk)
        `REPORT("F1_interface_instance_array", e_instarr,  1)
        `VPROBE(e_paramidx, posedge o.vd[W-2].clk)
        `REPORT("F2_parameterized_index",      e_paramidx, 1)

        `SVTEST_PASSFAIL
        $finish;
    end

endmodule


