//==========================================================================
// mwe_1109_00.sv -- ONE file, ONE issue: an unpacked value returned by a
//                   call is dropped at the actual-argument edge, together
//                   with the cousin defects that share that root cause.
//
// Simulator under test : xezim 0.11.5 (git bde5ae59)
// Reference simulator  : passes every check in this file.
// Verdict              : xezim TEST_FAIL, reference simulator TEST_PASS.
//--------------------------------------------------------------------------
// MAIN CAUSE -- P00, and the cousins P02..P10 + P17
//
//     loader.apply(keep, bank.buildCells(), EXTRA);  // xezim: list.size()==0
//
// The identical call routed through a temporary is fine:
//
//     tmp = bank.buildCells();
//     loader.apply(keep, tmp, EXTRA);                // xezim: list.size()==7
//
// The value the call produces is correct -- `.size()` on the call, nesting
// the call inside another call, and assigning it to a temporary all work
// (P11). It is lost ONLY on the actual-argument edge: the callee is handed
// an EMPTY container. P02..P10 and P17 show the loss is independent of the
// collection kind, the callee kind, positional-vs-named, constructors,
// arity, scope, and virtual dispatch.
//
// COUSIN 1 -- P13
//   An inline `'{...}` array literal as the actual keeps its SIZE but loses
//   its DATA. Different shape, same reason: the actual-to-formal binders
//   only understand storage-shaped actuals (names and member accesses).
//
// COUSIN 2 -- P14, P16
//   A derived class redeclaring a base dynamic array under the SAME name as
//   a fixed array: the fixed->dynamic conversion returns only the assigned
//   prefix. P15 shows that merely RENAMING the derived property fixes it,
//   which pins this on the shadowing rather than on the conversion. P16
//   feeds that producer through the actual edge, where both defects bite.
//
//--------------------------------------------------------------------------
// The run prints a MATRIX, one row per probe, with the verdict a buggy
// simulator is EXPECTED to produce. Controls (P01, P07, P11, P12, P15) are
// expected to PASS; if one of them fails the matrix says CONTROL BROKEN --
// that means the harness is at fault, not the simulator.
//
// Expected on xezim               : 13/13 bug probes firing, 5/5 controls
//                                   intact, TEST_FAIL count=21 / 44 checks.
// Expected on a correct simulator :  0/13 bug probes firing, 5/5 controls
//                                   intact, TEST_PASS / 44 checks.
//
// Run:  xezim -s tb mwe_1109_00.sv
//       reference simulator  -64bit -top tb mwe_1109_00.sv
//--------------------------------------------------------------------------
// All identifiers are deliberately generic placeholders (Cell, CellBank,
// WideBank, Loader, Host, ...). Nothing here is derived from, or names,
// any real design.
//==========================================================================

//--------------------------------------------------------------------------
// Harness. `PROBE opens a probe, `CHK records one check against the
// currently open probe, and print_matrix() prints the table. Deliberately
// macro-based and inline: the harness must not itself depend on the
// actual-argument mechanism that is under test. The run still terminates on
// the repo's TEST_PASS / TEST_FAIL count=N convention.
//--------------------------------------------------------------------------
`define PROBE(id, nm, claim, exp) \
  begin \
    cur                = (id); \
    probe_name[(id)]   = (nm); \
    probe_claim[(id)]  = (claim); \
    probe_expect[(id)] = (exp); \
    probe_checks[(id)] = 0; \
    probe_fails[(id)]  = 0; \
    $display(""); \
    $display("--- P%02d  %s", (id), (nm)); \
    $display("        %s", (claim)); \
  end

`define CHK(expr, msg) \
  begin \
    probe_checks[cur] = probe_checks[cur] + 1; \
    total_checks      = total_checks + 1; \
    if (!(expr)) begin \
      probe_fails[cur] = probe_fails[cur] + 1; \
      total_fails      = total_fails + 1; \
      $display("        FAIL: %s", msg); \
    end \
  end

//==========================================================================
// Shared element type. Every probe builds containers of this packed struct.
//==========================================================================
typedef struct packed {
    int unsigned tag;
    logic [63:0] payload;
} Cell;

//==========================================================================
// P00 -- PRIMARY REPRODUCER.
//
// Producer hierarchy:
//   CellBank  -- abstract base; holds a *dynamic* array and an empty
//                virtual factory hook.
//   WideBank  -- redeclares the same property name as a *fixed* array and
//                supplies the factory body (no `virtual` keyword, matching
//                the way the original code spells it).
//   FinalBank -- concrete leaf.
// Consumer:
//   Loader::apply -- records what it was actually handed. It RESETS its
//   bookkeeping on entry; without that the control run leaves state behind
//   and masks the defect.
//==========================================================================
localparam int NCELL = 7;
localparam bit [63:0] EXTRA = 64'h0000_0000_0000_002a;

typedef Cell CellList[];

virtual class CellBank;
    Cell cells[];

    virtual function CellList buildCells();
    endfunction
endclass

virtual class WideBank extends CellBank;
    Cell cells[NCELL];

    function CellList buildCells();
        begin
            cells[0].tag = 0; cells[0].payload[3:0] = 4'h1;
            cells[1].tag = 1; cells[1].payload[3:0] = 4'h0;
            cells[2].tag = 2; cells[2].payload[3:0] = 4'h0;
            cells[3].tag = 3; cells[3].payload[3:0] = 4'h0;
            cells[4].tag = 4; cells[4].payload[3:0] = 4'h0;
            cells[5].tag = 5; cells[5].payload[3:0] = 4'h0;
            cells[6].tag = 6; cells[6].payload[3:0] = 4'h0;
            return cells;
        end
    endfunction
endclass

class FinalBank extends WideBank;
endclass

class Loader;
    int unsigned seen;
    int unsigned seen_size;
    int unsigned hits;
    int unsigned extra_seen;

    task apply(bit mode, CellList list, bit [63:0] extra);
        int unsigned i;
        begin
            hits       = 0;
            seen       = 0;
            seen_size  = 0;
            extra_seen = extra[31:0];
            if (list.size() == 0) begin
                $display("        loader : EMPTY list handed over (size=0) -> nothing programmed");
                return;
            end
            seen_size = list.size();
            for (i = 0; i < list.size(); i = i + 1) begin
                if (list[i].payload[0]) hits++;
            end
            $display("        loader : programmed %0d cells (mode=%0b extra=%0d), hits=%0d",
                     list.size(), mode, extra[7:0], hits);
        end
    endtask
endclass

virtual class Host;
    CellBank bank;
    Loader   loader;
    bit      keep;

    function void bind_bank(CellBank b);
        this.bank = b;
    endfunction

    // The failing shape: bank.buildCells() *is* the actual argument.
    task configure();
        begin
            loader.apply(keep, bank.buildCells(), EXTRA);
        end
    endtask

    // The control: same call, same arguments, routed through a temporary.
    task configure_via_temp();
        CellList tmp;
        begin
            tmp = bank.buildCells();
            loader.apply(keep, tmp, EXTRA);
        end
    endtask
endclass

class FinalHost extends Host;
    FinalBank bank;                 // shadowing redeclaration, concrete type

    function void setup();
        FinalBank b;
        begin
            b      = new;
            bank   = b;
            super.bind_bank(b);
            loader = new;
            keep   = 0;
        end
    endfunction
endclass

//==========================================================================
// Cousin probe declarations.
//
// Most probes share one producer shape -- a class holding a fixed array of
// Cells and a method that returns them as a dynamic array. Blocks are grouped
// by that shape rather than in probe order: P01-P03, then P08-P09, then
// P04-P06 (queue / assoc / fixed), then P12-P13. P07, P10 and P11 follow the
// shadowing family P14-P17 below.
//
// Every probe gets its OWN copy of the producer and consumer under a unique
// cNN_ prefix, so no probe can mask or warm another -- the probes stay as
// independent as they were when each lived in its own file.
//==========================================================================

// ---- P01 (control) ------------------------------------------------------
typedef Cell c01_DynList[];
class c01_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c01_DynList mk();
        c01_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c01_Cons;
    int unsigned got;
    task take(input c01_DynList a);
        begin
            got = 0;
            got = a.size();
        end
    endtask
endclass

// ---- P02 ----------------------------------------------------------------
typedef Cell c02_DynList[];
class c02_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c02_DynList mk();
        c02_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c02_Cons;
    int unsigned got;
    int unsigned tag0;
    task take(input c02_DynList a);
        begin
            got  = 0;
            tag0 = 0;
            got  = a.size();
            tag0 = (a.size() == 0) ? 0 : a[0].tag;
        end
    endtask
endclass

// ---- P03 ----------------------------------------------------------------
typedef Cell c03_DynList[];
class c03_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c03_DynList mk();
        c03_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c03_Cons;
    function int count(input c03_DynList a);
        return a.size();
    endfunction
endclass

// ---- P08 ----------------------------------------------------------------
typedef Cell c08_DynList[];
class c08_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c08_DynList mk();
        c08_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c08_Cons;
    int unsigned got;
    function void named(input int n, input c08_DynList a);
        begin
            got = 0;
            got = a.size();
        end
    endfunction
endclass
class c08_Holder;
    int unsigned got;
    function new(input c08_DynList a);
        got = a.size();
    endfunction
endclass

// ---- P09 ----------------------------------------------------------------
typedef Cell c09_DynList[];
class c09_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c09_DynList mk();
        c09_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c09_Cons;
    int unsigned ga;
    int unsigned gb;
    task take(input c09_DynList a, input c09_DynList b);
        begin
            ga = 0;
            gb = 0;
            ga = a.size();
            gb = b.size();
        end
    endtask
endclass

//==========================================================================
// P04 -- queue ($) result.      P05 -- associative-array result.
// P06 -- fixed-size-array result.
//==========================================================================

// ---- P04 ----------------------------------------------------------------
typedef Cell c04_CellQ[$];
class c04_Prod;
    Cell store[2];
    function new();
        begin
            store[0].tag = 11; store[1].tag = 22;
        end
    endfunction
    function c04_CellQ mk();
        c04_CellQ q;
        begin
            q = '{store[0], store[1]};
            return q;
        end
    endfunction
endclass
class c04_Cons;
    int unsigned got;
    task take(input c04_CellQ a);
        begin
            got = 0;
            got = a.size();
        end
    endtask
endclass

// ---- P05 ----------------------------------------------------------------
typedef Cell c05_CellA[int];
class c05_Prod;
    Cell store[2];
    function new();
        begin
            store[0].tag = 11; store[1].tag = 22;
        end
    endfunction
    function c05_CellA mk();
        c05_CellA m;
        begin
            m[5] = store[0]; m[9] = store[1];
            return m;
        end
    endfunction
endclass
class c05_Cons;
    int unsigned got;
    task take(input c05_CellA a);
        begin
            got = 0;
            got = a.size();
        end
    endtask
endclass

// ---- P06 ----------------------------------------------------------------
typedef Cell c06_CellFix[4];
class c06_Prod;
    function c06_CellFix mk();
        c06_CellFix t;
        begin
            t[0].tag = 7; t[1].tag = 8; t[2].tag = 9; t[3].tag = 10;
            return t;
        end
    endfunction
endclass
class c06_Cons;
    int unsigned a0;
    int unsigned a3;
    task take(input c06_CellFix a);
        begin
            a0 = 0;
            a3 = 0;
            a0 = a[0].tag;
            a3 = a[3].tag;
        end
    endtask
endclass

// ---- P12 (control) -- plain property as the actual ----------------------
typedef Cell c12_DynList[];
class c12_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c12_DynList mk();
        c12_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c12_Cons;
    int unsigned got;
    int unsigned tag0;
    task take(input c12_DynList a);
        begin
            got  = 0;
            tag0 = 0;
            got  = a.size();
            tag0 = (a.size() == 0) ? 0 : a[0].tag;
        end
    endtask
endclass

// ---- P13 -- inline array-literal actual ---------------------------------
typedef Cell c13_DynList[];
class c13_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c13_DynList mk();
        c13_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass
class c13_Cons;
    int unsigned got;
    int unsigned t0;
    int unsigned t1;
    task take(input c13_DynList a);
        begin
            got = 0; t0 = 0; t1 = 0;
            got = a.size();
            t0  = (a.size() == 0) ? 0 : a[0].tag;
            t1  = (a.size() < 2)  ? 0 : a[1].tag;
        end
    endtask
endclass

//==========================================================================
// P14 / P15 / P16 -- the shadowing family.
//
// BankBase declares `items` as a dynamic array; BankLeaf REDECLARES THE
// SAME NAME as a fixed array and returns it from build(). The fixed->dynamic
// conversion then hands back only the assigned prefix. P15 is identical
// except the derived property is RENAMED, and it is correct -- that is what
// proves the defect is the shadowing, not the conversion.
//==========================================================================

// ---- P14 ----------------------------------------------------------------
typedef Cell c14_DynList[];
virtual class c14_BankBase;
    Cell items[];
    virtual function c14_DynList build();
    endfunction
endclass
class c14_BankLeaf extends c14_BankBase;
    Cell items[5];
    function c14_DynList build();
        begin
            items[0].tag = 100;
            items[1].tag = 101;
            return items;                 // 2 of 5 elements written
        end
    endfunction
endclass

// ---- P15 (control: same code, derived property renamed) ------------------
typedef Cell c15_DynList[];
virtual class c15_BankBase;
    Cell items[];
    virtual function c15_DynList build();
    endfunction
endclass
class c15_BankLeaf extends c15_BankBase;
    Cell items2[5];
    function c15_DynList build();
        begin
            items2[0].tag = 100;
            items2[1].tag = 101;
            return items2;                // 2 of 5 elements written
        end
    endfunction
endclass

// ---- P16 -- P14's producer consumed through the actual edge --------------
typedef Cell c16_DynList[];
virtual class c16_BankBase;
    Cell items[];
    virtual function c16_DynList build();
    endfunction
endclass
class c16_BankLeaf extends c16_BankBase;
    Cell items[5];
    function c16_DynList build();
        begin
            items[0].tag = 100;
            items[1].tag = 101;
            return items;
        end
    endfunction
endclass
class c16_Cons;
    int unsigned got;
    int unsigned t0;
    task take(input c16_DynList a);
        begin
            got = 0; t0 = 0;
            got = a.size();
            t0  = (a.size() == 0) ? 0 : a[0].tag;
        end
    endtask
endclass

//==========================================================================
// P17 -- the producer override is reached by VIRTUAL DISPATCH through a
// base-typed handle, then the result is passed directly as an actual.
//==========================================================================
typedef Cell c17_DynList[];
virtual class c17_ProdBase;
    virtual function c17_DynList mk();
        c17_DynList t;
        begin
            t = new[1];
            t[0].tag = 0;
            return t;
        end
    endfunction
endclass
class c17_ProdLeaf extends c17_ProdBase;
    Cell store[4];
    function new();
        begin
            store[0].tag = 4; store[1].tag = 5; store[2].tag = 6; store[3].tag = 7;
        end
    endfunction
    virtual function c17_DynList mk();
        c17_DynList t;
        begin
            t = new[4];
            t[0] = store[0]; t[1] = store[1]; t[2] = store[2]; t[3] = store[3];
            return t;
        end
    endfunction
endclass
class c17_Cons;
    int unsigned got;
    int unsigned t3;
    task take(input c17_DynList a);
        begin
            got = 0; t3 = 0;
            got = a.size();
            t3  = (a.size() < 4) ? 0 : a[3].tag;
        end
    endtask
endclass

//==========================================================================
// P07 (control) -- scalar / string / packed-struct results passed directly.
// P10 -- module-scope producer and consumer (see module tb below).
// P11 (control) -- the same call in the OTHER expression contexts.
//==========================================================================
class c07_Prod2;
    function int    mk_int();  return 42;      endfunction
    function string mk_str();  return "hello"; endfunction
    function Cell   mk_cell();
        Cell t;
        begin
            t.tag = 99;
            return t;
        end
    endfunction
endclass
class c07_Cons;
    int    gi;
    string gs;
    int    gc;
    task take(input int i, input string s, input Cell c);
        begin
            gi = 0; gs = ""; gc = 0;
            gi = i; gs = s; gc = c.tag;
        end
    endtask
endclass

typedef Cell c10_DynList[];
typedef Cell c11_DynList[];

class c11_Prod;
    Cell prop[3];
    function new();
        begin
            prop[0].tag = 1; prop[1].tag = 2; prop[2].tag = 3;
        end
    endfunction
    function c11_DynList mk();
        c11_DynList t;
        begin
            t = new[3];
            t[0] = prop[0]; t[1] = prop[1]; t[2] = prop[2];
            return t;
        end
    endfunction
endclass

//==========================================================================
// Testbench: the matrix state, one run_pXX task per probe, and the initial
// block that runs them all and prints the report.
//==========================================================================

module tb;
    localparam int NPROBE = 18;

    string probe_name[NPROBE];
    string probe_claim[NPROBE];
    int    probe_expect[NPROBE];   // 0 = must PASS (control), 1 = must FAIL (bug)
    int    probe_checks[NPROBE];
    int    probe_fails[NPROBE];
    int    cur;
    int    total_checks;
    int    total_fails;

    task print_matrix();
        int    i;
        int    bug_total;
        int    bug_seen;
        int    ctl_total;
        int    ctl_ok;
        int    obs;
        int    exp;
        string status;
        begin
            bug_total = 0;
            bug_seen  = 0;
            ctl_total = 0;
            ctl_ok    = 0;
            $display("");
            $display("=================================================================================");
            $display(" PROBE MATRIX -- one row per reproducer in this file (%0d probes)", NPROBE);
            $display(" 'expect' is what a BUGGY simulator must do: FAIL a bug probe, PASS a control.");
            $display("---------------------------------------------------------------------------------");
            $display("   id   probe                        checks   fails   observed  expect   status");
            $display("---------------------------------------------------------------------------------");
            for (i = 0; i < NPROBE; i = i + 1) begin
                obs = (probe_fails[i] == 0) ? 0 : 1;
                exp = probe_expect[i];
                if (exp == 0) begin
                    ctl_total = ctl_total + 1;
                    if (obs == 0) begin
                        status = "control ok";
                        ctl_ok = ctl_ok + 1;
                    end else begin
                        status = "CONTROL BROKEN";
                    end
                end else begin
                    bug_total = bug_total + 1;
                    if (obs == 1) begin
                        status    = "bug reproduced";
                        bug_seen  = bug_seen + 1;
                    end else begin
                        status = "correct here";
                    end
                end
                $display("   P%02d   %-26s %6d %7d      %-6s  %-6s   %s",
                         i, probe_name[i], probe_checks[i], probe_fails[i],
                         (obs == 0) ? "PASS" : "FAIL",
                         (exp == 0) ? "PASS" : "FAIL",
                         status);
            end
            $display("---------------------------------------------------------------------------------");
            $display("   controls intact    : %0d/%0d", ctl_ok, ctl_total);
            $display("   bug probes firing  : %0d/%0d", bug_seen, bug_total);
            if (ctl_ok == ctl_total) begin
                $display("   harness integrity  : OK -- every control passed, so the failures");
                $display("                        below the line are the simulator, not this file.");
            end else begin
                $display("   harness integrity  : BROKEN -- a control failed; distrust the bug rows.");
            end
            $display("   total checks       : %0d      total failures : %0d", total_checks, total_fails);
            $display("   (a correct simulator reports 0/%0d bug probes firing and %0d/%0d controls intact)",
                     bug_total, ctl_total, ctl_total);
            $display("=================================================================================");
        end
    endtask

    // ---------------------------------------------------------------- P00
    FinalHost h00;
    task run_p00();
        begin
            `PROBE(0, "primary_reproducer", "call result passed directly as an actual arrives EMPTY", 1)
            h00 = new;
            h00.setup();
            h00.configure_via_temp();
            `CHK(h00.loader.seen_size  == NCELL,   "control: loader must be handed NCELL cells")
            `CHK(h00.loader.hits       == 1,       "control: exactly one cell carries the payload bit")
            `CHK(h00.loader.extra_seen == 32'h2a,  "control: trailing scalar actual arrives intact")
            h00.configure();
            `CHK(h00.loader.seen_size  == NCELL,   "bug: loader must be handed NCELL cells")
            `CHK(h00.loader.hits       == 1,       "bug: exactly one cell carries the payload bit")
            `CHK(h00.loader.extra_seen == 32'h2a,  "bug: trailing scalar actual arrives intact")
        end
    endtask

    // ---------------------------------------------------------------- P01
    c01_Prod p01; c01_Cons k01; c01_DynList t01;
    task run_p01();
        begin
            `PROBE(1, "temp_control", "CONTROL: same call routed through a temporary is correct", 0)
            p01 = new; k01 = new;
            t01 = p01.mk();
            k01.take(t01);
            `CHK(k01.got == 3, "temporary-routed actual must carry 3 elements")
        end
    endtask

    // ---------------------------------------------------------------- P02
    c02_Prod p02; c02_Cons k02;
    task run_p02();
        begin
            `PROBE(2, "direct_actual", "dynamic-array result of a call used directly as an actual", 1)
            p02 = new; k02 = new;
            k02.take(p02.mk());
            `CHK(k02.got  == 3, "direct actual must carry 3 elements")
            `CHK(k02.tag0 == 1, "direct actual must carry the element data")
        end
    endtask

    // ---------------------------------------------------------------- P03
    c03_Prod p03; c03_Cons k03;
    task run_p03();
        begin
            `PROBE(3, "function_callee", "callee is a FUNCTION not a task -- same loss", 1)
            p03 = new; k03 = new;
            `CHK(k03.count(p03.mk()) == 3, "function callee must see 3 elements")
        end
    endtask

    // ---------------------------------------------------------------- P04
    c04_Prod p04; c04_Cons k04;
    task run_p04();
        begin
            `PROBE(4, "queue_actual", "queue ($) result of a call used directly as an actual", 1)
            p04 = new; k04 = new;
            k04.take(p04.mk());
            `CHK(k04.got == 2, "queue actual must carry 2 elements")
        end
    endtask

    // ---------------------------------------------------------------- P05
    c05_Prod p05; c05_Cons k05;
    task run_p05();
        begin
            `PROBE(5, "assoc_actual", "associative-array result of a call used directly as an actual", 1)
            p05 = new; k05 = new;
            k05.take(p05.mk());
            `CHK(k05.got == 2, "associative-array actual must carry 2 entries")
        end
    endtask

    // ---------------------------------------------------------------- P06
    c06_Prod p06; c06_Cons k06;
    task run_p06();
        begin
            `PROBE(6, "fixed_actual", "fixed-size-array result of a call used directly as an actual", 1)
            p06 = new; k06 = new;
            k06.take(p06.mk());
            `CHK(k06.a0 == 7,  "fixed-array actual must carry element 0")
            `CHK(k06.a3 == 10, "fixed-array actual must carry element 3")
        end
    endtask

    // ---------------------------------------------------------------- P07
    c07_Prod2 p07; c07_Cons k07;
    task run_p07();
        begin
            `PROBE(7, "scalar_control", "CONTROL: scalar / string / packed-struct actuals survive", 0)
            p07 = new; k07 = new;
            k07.take(p07.mk_int(), p07.mk_str(), p07.mk_cell());
            `CHK(k07.gi == 42,      "scalar int actual survives")
            `CHK(k07.gs == "hello", "string actual survives")
            `CHK(k07.gc == 99,      "packed-struct actual survives")
        end
    endtask

    // ---------------------------------------------------------------- P08
    c08_Prod p08; c08_Cons k08; c08_Holder d08;
    task run_p08();
        begin
            `PROBE(8, "named_and_ctor", "loss also hits named (.a(expr)) actuals and new() actuals", 1)
            p08 = new; k08 = new;
            k08.named(.n(7), .a(p08.mk()));
            `CHK(k08.got == 3, "named actual must carry 3 elements")
            d08 = new(p08.mk());
            `CHK(d08.got == 3, "constructor actual must carry 3 elements")
        end
    endtask

    // ---------------------------------------------------------------- P09
    c09_Prod p09; c09_Cons k09; c09_DynList t09;
    task run_p09();
        begin
            `PROBE(9, "two_actuals", "one temporary actual and one call actual; only the latter empties", 1)
            p09 = new; k09 = new;
            t09 = p09.mk();
            k09.take(t09, p09.mk());
            `CHK(k09.ga == 3, "temporary actual must carry 3 elements")
            `CHK(k09.gb == 3, "call actual must carry 3 elements")
        end
    endtask

    // ---------------------------------------------------------------- P10
    // Producer and consumer live at MODULE scope here, not in a class.
    int unsigned got10;
    function c10_DynList c10_mod_mk();
        c10_DynList t;
        begin
            t = new[5];
            t[0].tag = 1;
            return t;
        end
    endfunction
    task c10_mod_take(input c10_DynList a);
        begin
            got10 = 0;
            got10 = a.size();
        end
    endtask

    task run_p10();
        begin
            `PROBE(10, "module_scope", "module-scope function returning a dynamic array passed directly", 1)
            c10_mod_take(c10_mod_mk());
            `CHK(got10 == 5, "module-scope call actual must carry 5 elements")
        end
    endtask

    // ---------------------------------------------------------------- P11
    c11_Prod p11; c11_DynList t11;
    function int c11_echo(input int n);
        return n;
    endfunction
    task run_p11();
        begin
            `PROBE(11, "other_expr_control", "CONTROL: same call as a bare expression / nested / via temp", 0)
            p11 = new;
            `CHK(p11.mk().size() == 3,             "built-in method on the call result works")
            `CHK(c11_echo(p11.mk().size()) == 3,   "call nested in another call works")
            t11 = p11.mk();
            `CHK(t11.size() == 3,                  "assignment from the call works")
            `CHK(t11[2].tag == 3,                  "assigned result keeps its element data")
        end
    endtask

    // ---------------------------------------------------------------- P12
    c12_Prod p12; c12_Cons k12;
    task run_p12();
        begin
            `PROBE(12, "property_control", "CONTROL: a plain property (no call) as the actual is fine", 0)
            p12 = new; k12 = new;
            p12.prop = p12.mk();              // property now holds a 3-element array
            k12.take(p12.prop);
            `CHK(k12.got  == 3, "property actual must carry 3 elements")
            `CHK(k12.tag0 == 1, "property actual keeps its element data")
        end
    endtask

    // ---------------------------------------------------------------- P13
    c13_Prod p13; c13_Cons k13; c13_DynList l13;
    task run_p13();
        begin
            `PROBE(13, "literal_actual", "COUSIN: inline '{...} array-literal actual keeps size loses data", 1)
            p13 = new; k13 = new;
            l13 = p13.mk();
            k13.take(l13);
            `CHK(k13.t0 == 1 && k13.t1 == 2, "temporary-routed literal keeps its data")
            k13.take('{l13[0], l13[1]});
            `CHK(k13.got == 2, "inline literal keeps its size")
            `CHK(k13.t0 == 1,  "inline literal keeps element 0")
            `CHK(k13.t1 == 2,  "inline literal keeps element 1")
        end
    endtask

    // ---------------------------------------------------------------- P14
    c14_BankLeaf b14; c14_BankBase g14; c14_DynList r14;
    task run_p14();
        begin
            `PROBE(14, "shadowed_fixed", "COUSIN: derived fixed array shadows a base dynamic array of the SAME name", 1)
            b14 = new;
            g14 = b14;
            r14 = g14.build();
            `CHK(r14.size() == 5,    "fixed array of 5 must convert to size 5")
            `CHK(r14[0].tag == 100,  "conversion keeps element 0")
            `CHK(r14[1].tag == 101,  "conversion keeps element 1")
        end
    endtask

    // ---------------------------------------------------------------- P15
    c15_BankLeaf b15; c15_BankBase g15; c15_DynList r15;
    task run_p15();
        begin
            `PROBE(15, "shadow_rename_control", "CONTROL: identical to P14 but the derived property is renamed", 0)
            b15 = new;
            g15 = b15;
            r15 = g15.build();
            `CHK(r15.size() == 5,    "fixed array of 5 must convert to size 5")
            `CHK(r15[0].tag == 100,  "conversion keeps element 0")
            `CHK(r15[1].tag == 101,  "conversion keeps element 1")
        end
    endtask

    // ---------------------------------------------------------------- P16
    c16_BankLeaf b16; c16_BankBase g16; c16_Cons k16; c16_DynList t16;
    task run_p16();
        begin
            `PROBE(16, "shadowed_through_actual", "COUSIN: P14 producer consumed through the actual edge", 1)
            b16 = new; g16 = b16; k16 = new;
            t16 = g16.build();
            k16.take(t16);
            `CHK(k16.got == 5, "temporary path: fixed array of 5 must be size 5")
            k16.take(g16.build());
            `CHK(k16.got == 5,  "direct actual: fixed array of 5 must be size 5")
            `CHK(k16.t0 == 100, "direct actual: element 0 must survive")
        end
    endtask

    // ---------------------------------------------------------------- P17
    c17_ProdLeaf w17; c17_ProdBase q17; c17_Cons k17; c17_DynList u17;
    task run_p17();
        begin
            `PROBE(17, "virtual_dispatch", "COUSIN: override reached by virtual dispatch through a base-typed handle", 1)
            w17 = new; q17 = w17; k17 = new;
            u17 = q17.mk();
            k17.take(u17);
            `CHK(k17.got == 4 && k17.t3 == 7, "virtual dispatch via temporary is fine")
            k17.take(q17.mk());
            `CHK(k17.got == 4, "virtual dispatch via direct actual must carry 4")
            `CHK(k17.t3 == 7,  "virtual dispatch via direct actual keeps its data")
        end
    endtask

    // ==================================================================
    initial begin
        total_checks = 0;
        total_fails  = 0;
        cur          = 0;

        run_p00();
        run_p01();
        run_p02();
        run_p03();
        run_p04();
        run_p05();
        run_p06();
        run_p07();
        run_p08();
        run_p09();
        run_p10();
        run_p11();
        run_p12();
        run_p13();
        run_p14();
        run_p15();
        run_p16();
        run_p17();

        if (total_fails == 0) begin
            $display("TEST_PASS");
        end else begin
            $display("TEST_FAIL count=%0d", total_fails);
            $fatal(1);
        end
        $finish;
    end
endmodule
