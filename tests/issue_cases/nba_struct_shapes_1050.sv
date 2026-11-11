// nba_struct_shapes_1050.sv -- MWE + cousin-shape sweep for an xezim NBA bug.
//
// Core shape: a PARTIAL-MEMBER nonblocking assignment (NBA) followed, in the
// SAME timestep, by a WHOLE-STRUCT NBA to the same variable loses the
// whole-struct value of that member (the partial value sticks), while all
// other members correctly take the whole-struct value.
//
// Found in a production environment as an interface request bus whose valid
// bit never asserted: a driver deasserted <bus>.vld with a partial-member
// NBA and, on a zero-delay path, placed the next request with a whole-struct
// NBA in the same timestep. The reference simulator applies both NBAs in
// scheduling order (whole-struct last -> vld=1); xezim leaves vld=0.
//
// This file sweeps the core shape plus cousin shapes to map the blast
// radius. Expectations follow IEEE 1800-2017 10.4.1/4.7: NBAs to the same
// variable in the same timestep are applied in scheduling order, so the
// last-scheduled update (the whole-struct NBA) wins.
//
// Uses the SVTEST macros from the xezim repo test suite
// (sv_compliance/common/svtest_defs.svh): `SVTEST_INIT, `SVTEST_CHECK,
// `SVTEST_PASSFAIL. A clean run prints TEST_PASS; on xezim every diverging
// shape prints a "FAIL: sNN ..." line and the run ends with TEST_FAIL.
//
// All identifiers are generic (no production names); run from tests/issue_cases/:
//   xezim nba_struct_shapes_1050.sv
//   <reference simulator> -sv nba_struct_shapes_1050.sv

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


package nba_types;
  // Member count/widths mirror the production struct; names are generic.
  typedef struct packed {
    logic        vld;
    logic        wr;
    logic [39:0] addr;
    logic [2:0]  tag;
    logic [2:0]  sz;
    logic [7:0]  len;
    logic        attr;
    logic [1:0]  mode;
    logic        hold;
  } pkt_t;

  typedef struct packed {
    logic        vld;
    logic [31:0] data;
    logic [3:0]  strb;
    logic        last;
  } beat_t;

  // Nested-struct cousin.
  typedef struct packed {
    logic        vld;
    logic [2:0]  tag;
  } hdr_t;
  typedef struct packed {
    hdr_t        hdr;
    logic [39:0] addr;
  } npkt_t;

  // Unpacked-struct cousin.
  typedef struct {
    logic        vld;
    logic [39:0] addr;
    logic [7:0]  len;
  } upkt_t;

  // Deeper nesting and packed-union cousins (s18, s17).
  typedef struct packed {
    hdr_t        hdr;
    hdr_t        hdr2;
    logic [39:0] addr;
  } npkt3_t;

  typedef union packed {
    logic [31:0]     w;
    logic [3:0][7:0] b;
  } un_t;

  // Package-scope variable target for the scope sweep (s44).
  pkt_t pkg_pkt;
endpackage

interface chan_if (input logic clk);
  import nba_types::*;
  pkt_t  bus_a;
  beat_t bus_b;
  npkt_t bus_n;
  // Modport view for the scope sweep (s57, s58).
  modport drv (output bus_a);
endinterface

// Submodule target for the hierarchical-reference shape (s24).
module nba_sub;
  import nba_types::*;
  pkt_t mv;
endmodule

module test_nba_struct_shapes;

  import nba_types::*;

  `SVTEST_INIT

  logic clk = 1'b0;
  always #5 clk = ~clk;

  chan_if ci (clk);   // producer-side (driven) interface
  chan_if di (clk);   // downstream (sampling) interface

  // Downstream pin path as in production: only the valid bit crosses.
  logic pin_a_vld, pin_b_vld;
  assign pin_a_vld    = ci.bus_a.vld;
  assign pin_b_vld    = ci.bus_b.vld;
  assign di.bus_a.vld = pin_a_vld;
  assign di.bus_b.vld = pin_b_vld;

  int a_vld_events = 0, b_vld_events = 0;
  int sample_en    = 0;
  initial forever @(posedge clk iff di.bus_a.vld) if (sample_en == 1) a_vld_events++;
  initial forever @(posedge clk iff di.bus_b.vld) if (sample_en == 2) b_vld_events++;

  // Cousin targets outside the interface.
  pkt_t        mv;      // module-scope struct variable
  upkt_t       up;      // unpacked struct variable
  pkt_t        arr [2]; // unpacked array of structs
  logic [59:0] vec;     // flat vector, same width as pkt_t (60 bits)

  // ------------------------------------------------------------------
  // Extra scopes for the s17..s43 cousin sweep (see README).
  // ------------------------------------------------------------------
  chan_if   cia [2] (clk);   // interface ARRAY                 (s23)
  nba_sub   sub ();          // submodule target                (s24)

  // Class-object property target (s25, s40..s43).
  class holder;
    pkt_t       p;
    logic [63:0] vec;
  endclass
  holder hdl, hdl2;      // hdl2 is used only by s40..s42 (order-independent)
  pkt_t  mv43;           // module-variable NBA width control (s43)

  un_t      uni;             // packed union                     (s17)
  npkt3_t   n3;              // three-level nested packed struct (s18)
  pkt_t     q19;             // struct-member part-select        (s19)
  logic [59:0] v20, v21, v22; // indexed part-selects / concat    (s20..s22)
  pkt_t     q26, q27, q28, q29, q30, q31, q32, q33;
  pkt_t     arr30 [4];       // foreach-issued partial NBAs      (s30)
  pkt_t     q34, q35, q36, q37, q38, q39; // process/code-path axis
  int       sel = 1;         // variable base for +: / -: selects
  int       en34 = 0, en35 = 0, en36 = 0; // enables for the always-path controls

  // Golden values (positional field order: vld,wr,addr,tag,sz,len,attr,mode,hold).
  pkt_t  PKT  = '{1'b1, 1'b1, 40'h0000_C0DE, 3'd2, 3'd2, 8'd0, 1'b0, 2'd1, 1'b0};
  pkt_t  PKT0 = '{1'b0, 1'b1, 40'h0000_BA0B, 3'd1, 3'd1, 8'd3, 1'b1, 2'd2, 1'b1};
  beat_t BEAT = '{1'b1, 32'hD00D_F00D, 4'hF, 1'b1};
  npkt_t NPKT = '{'{1'b1, 3'd5}, 40'h0000_FACE};
  upkt_t UPKT = '{1'b1, 40'h0000_0FF1, 8'd7};
  npkt3_t NPKT3 = '{'{1'b1, 3'd5}, '{1'b1, 3'd6}, 40'h0000_FACE};
  un_t    UN    = 32'hDEAD_BEEF;
  // PKT with len replaced by 8'h11 -- expected result of the s27 triple.
  pkt_t  PKT_L11 = '{1'b1, 1'b1, 40'h0000_C0DE, 3'd2, 3'd2, 8'h11, 1'b0, 2'd1, 1'b0};
  logic [63:0] VEC64 = 64'hDEAD_BEEF_CAFE_F00D; // 64-bit class-property golden
  logic [59:0] pkt_bits; // flat-image of PKT, built in the initial block

  // ------------------------------------------------------------------
  // Extra scopes for the s44..s59 scope sweep (see README).
  // ------------------------------------------------------------------

  // Generate-block variable target (s45).
  generate
    if (1) begin : gscope
      pkt_t gen_pkt;
    end
  endgenerate

  // Width sweep: the NBA target width of a HANDLE-qualified property falls
  // back to the 32-bit default, so a wider RHS is truncated and a `real`
  // RHS is converted to a 32-bit integer (s47..s50).  `this.`-qualified
  // and bare-name targets keep the declared width (s51, s52).
  class width_holder;
    logic [31:0] w32;
    logic [32:0] w33;
    logic [63:0] w64;
    real         r;
    task automatic nba_this_wide();
      this.w64 <= 64'h0123_4567_89AB_CDEF;
      this.r   <= -1.5;
    endtask
    task automatic nba_bare_real();
      r <= -1.5;
    endtask
  endclass
  width_holder wh;

  // Class STATIC property target (s53, s54, s59).
  class static_holder;
    static pkt_t        sp;
    static logic [63:0] sv;
  endclass

  // super.-qualified property target (s55, s56).
  class base_holder;
    pkt_t bp;
  endclass
  class derived_holder extends base_holder;
    task automatic nba_super_whole(input pkt_t v);
      super.bp <= v;
    endtask
    task automatic blk_super_whole(input pkt_t v);
      super.bp = v;
    endtask
    task automatic nba_super_partial_whole(input pkt_t v);
      super.bp.vld <= 1'b0;
      super.bp     <= v;
    endtask
  endclass
  derived_holder dh;

  // ------------------------------------------------------------------
  // Class-based cousin: the exact production placement pattern
  // (virtual interface + credit/queue mailboxes + zero-delay repeat).
  // ------------------------------------------------------------------
  class txn_info;
    pkt_t      pkt;
    int unsigned place_delay;
  endclass

  class beat_info;
    beat_t     beat;
    int unsigned place_delay;
  endclass

  class producer;
    virtual chan_if     vif;
    mailbox #(txn_info)  txn_q  = new();
    mailbox #(bit)       txn_cr = new();
    mailbox #(beat_info) beat_q = new();
    mailbox #(bit)       beat_cr = new();

    // Mirror of the production placement loop: partial-member deassert,
    // blocking queue get, zero-iteration delay, whole-struct place.
    task run_txn(int unsigned n_placements);
      txn_info ti;
      bit      dummy;
      repeat (n_placements) begin
        @(posedge vif.clk);
        if ((txn_cr.num() > 0) && (txn_q.num() > 0)) begin
          vif.bus_a.vld <= 1'b0;                      // partial-member NBA
          txn_q.get(ti);
          repeat (ti.place_delay) @(posedge vif.clk); // 0 -> no wait
          vif.bus_a <= ti.pkt;                        // whole-struct NBA, same timestep
          txn_cr.get(dummy);
        end else begin
          vif.bus_a.vld <= 1'b0;
        end
      end
    endtask

    // Control: the production data-beat path (whole+whole NBAs only).
    task run_beat(int unsigned n_placements);
      beat_info bi;
      bit       dummy;
      repeat (n_placements) begin
        @(posedge vif.clk);
        if ((beat_cr.num() > 0) && (beat_q.num() > 0)) begin
          vif.bus_b <= '0;                            // whole-struct NBA
          beat_q.get(bi);
          repeat (bi.place_delay) @(posedge vif.clk);
          vif.bus_b <= bi.beat;                       // whole-struct NBA
          beat_cr.get(dummy);
        end else begin
          vif.bus_b.vld <= 1'b0;
        end
      end
    endtask
  endclass

  // ------------------------------------------------------------------
  // Separate processes for the process / code-path axis (s34..s36) and
  // the function-issued partial NBA (s29).
  // ------------------------------------------------------------------
  always @(posedge clk)    if (en34) begin q34.vld <= 1'b0; q34 <= PKT; end
  always_ff @(posedge clk) if (en35) begin q35.vld <= 1'b0; q35 <= PKT; end
  initial forever @(posedge clk) if (en36) begin q36.vld <= 1'b0; q36 <= PKT; end

  function automatic void fn29();
    q29.vld <= 1'b0;   // partial-member NBA issued from inside a function
  endfunction

  // ------------------------------------------------------------------
  // Helpers
  // ------------------------------------------------------------------
  task dump_pkt(input string s, input pkt_t p);
    $display("SHAPE %s: vld=%b wr=%b addr=%h tag=%h sz=%h len=%h attr=%b mode=%b hold=%b",
             s, p.vld, p.wr, p.addr, p.tag, p.sz, p.len, p.attr, p.mode, p.hold);
  endtask

  task dump_upkt(input string s, input upkt_t p);
    $display("SHAPE %s: vld=%b addr=%h len=%h", s, p.vld, p.addr, p.len);
  endtask

  // ------------------------------------------------------------------
  // Shape tasks. Each drives NBAs at a posedge and checks the settled
  // value at the following negedge (no race with the NBA region).
  // ------------------------------------------------------------------

  // s01 CONTROL: whole + whole (the shape that always worked).
  task s01_whole_whole;
    @(posedge clk) ci.bus_a <= '0;
                   ci.bus_a <= PKT;
    @(negedge clk);
    dump_pkt("s01", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s01 whole+whole: final value should be PKT")
  endtask

  // s02 CORE BUG: partial-member vld deassert + same-timestep whole-struct.
  task s02_partial_vld_then_whole;
    @(posedge clk) ci.bus_a.vld <= 1'b0;
                   ci.bus_a      <= PKT;
    @(negedge clk);
    dump_pkt("s02", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s02 partial-vld+whole: whole-struct NBA must win for vld")
  endtask

  // s03 COUSIN: reverse order (whole first, partial-member last).
  task s03_whole_then_partial_vld;
    @(posedge clk) ci.bus_a <= PKT0;
                   ci.bus_a.vld <= 1'b1;
    @(negedge clk);
    dump_pkt("s03", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a.vld == 1'b1 && ci.bus_a.addr == PKT0.addr,
                 "s03 whole+partial-vld: partial-member NBA must win for vld")
  endtask

  // s04 COUSIN: partial-member on a different member (addr).
  task s04_partial_addr_then_whole;
    @(posedge clk) ci.bus_a.addr <= 40'hFFFF_FFFF_FF;
                   ci.bus_a      <= PKT;
    @(negedge clk);
    dump_pkt("s04", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s04 partial-addr+whole: whole-struct NBA must win for addr")
  endtask

  // s05 COUSIN: two partial members then whole-struct.
  task s05_partial_multi_then_whole;
    @(posedge clk) ci.bus_a.vld <= 1'b0;
                   ci.bus_a.len <= 8'hFF;
                   ci.bus_a     <= PKT;
    @(negedge clk);
    dump_pkt("s05", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s05 partial-multi+whole: whole-struct NBA must win")
  endtask

  // s06 COUSIN: partial and whole in DIFFERENT timesteps (worked in production).
  task s06_partial_then_whole_next_cycle;
    @(posedge clk) ci.bus_a.vld <= 1'b0;
    @(posedge clk) ci.bus_a <= PKT;
    @(negedge clk);
    dump_pkt("s06", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s06 partial+whole next cycle: whole-struct NBA must win")
  endtask

  // s07 COUSIN: flat vector, single-bit NBA then whole-vector NBA.
  // Bit 59 is the vld position and is 1 in the whole value -> discriminates.
  task s07_vec_bit_then_whole;
    @(posedge clk) vec[59] <= 1'b0;
                   vec     <= pkt_bits;
    @(negedge clk);
    $display("SHAPE s07: vec=%h bit59=%b", vec, vec[59]);
    `SVTEST_CHECK(vec == pkt_bits, "s07 vec-bit+whole: whole-vector NBA must win")
  endtask

  // s08 COUSIN: flat vector, multi-bit part-select NBA then whole-vector NBA.
  // Bits 33:30 map to PKT.addr[15:12]=4'b1100 (nonzero in the whole value).
  task s08_vec_partsel_then_whole;
    @(posedge clk) vec[33:30] <= 4'b0011;
                   vec        <= pkt_bits;
    @(negedge clk);
    $display("SHAPE s08: vec=%h bits33_30=%b", vec, vec[33:30]);
    `SVTEST_CHECK(vec == pkt_bits, "s08 vec-partsel+whole: whole-vector NBA must win")
  endtask

  // s09 COUSIN: nested struct, member-of-member NBA then whole-struct NBA.
  task s09_nested_member_then_whole;
    @(posedge clk) ci.bus_n.hdr.vld <= 1'b0;
                   ci.bus_n         <= NPKT;
    @(negedge clk);
    $display("SHAPE s09: hdr.vld=%b hdr.tag=%h addr=%h",
             ci.bus_n.hdr.vld, ci.bus_n.hdr.tag, ci.bus_n.addr);
    `SVTEST_CHECK(ci.bus_n == NPKT, "s09 nested-member+whole: whole-struct NBA must win")
  endtask

  // s10 COUSIN: module-scope variable (no interface involved).
  task s10_module_var_partial_then_whole;
    @(posedge clk) mv.vld <= 1'b0;
                   mv     <= PKT;
    @(negedge clk);
    dump_pkt("s10", mv);
    `SVTEST_CHECK(mv == PKT, "s10 module-var partial+whole: whole-struct NBA must win")
  endtask

  // s11 COUSIN: unpacked struct, member NBA then whole-struct NBA.
  task s11_unpacked_partial_then_whole;
    @(posedge clk) up.vld <= 1'b0;
                   up     <= UPKT;
    @(negedge clk);
    dump_upkt("s11", up);
    `SVTEST_CHECK(up.vld == 1'b1 && up.addr == UPKT.addr && up.len == UPKT.len,
                 "s11 unpacked partial+whole: whole-struct NBA must win")
  endtask

  // s12 COUSIN: unpacked array of structs, element member NBA then element whole NBA.
  task s12_array_elem_partial_then_whole;
    @(posedge clk) arr[0].vld <= 1'b0;
                   arr[1]     <= PKT0;
                   arr[0]     <= PKT;
    @(negedge clk);
    dump_pkt("s12[0]", arr[0]);
    dump_pkt("s12[1]", arr[1]);
    `SVTEST_CHECK(arr[0] == PKT && arr[1] == PKT0,
                 "s12 array-elem partial+whole: whole-struct NBA must win")
  endtask

  // s13 CORE BUG via the exact production path: class + virtual interface +
  // mailboxes + zero-delay repeat, sampled through the downstream pin.
  task s13_vif_class_partial_then_whole;
    producer   drv;
    txn_info   ti;
    drv = new();
    drv.vif = ci;
    repeat (3) begin
      ti = new();
      ti.pkt = PKT;
      ti.place_delay = 0;   // zero-delay path: both NBAs land in one timestep
      drv.txn_q.put(ti);
      drv.txn_cr.put(1'b0);
    end
    sample_en = 1;
    drv.run_txn(3);
    repeat (2) @(posedge clk);   // let downstream samples of the last placements fire
    @(negedge clk);
    sample_en = 0;
    dump_pkt("s13", ci.bus_a);
    $display("SHAPE s13: a_vld_events=%0d", a_vld_events);
    `SVTEST_CHECK(ci.bus_a == PKT, "s13 vif/class partial+whole: whole-struct NBA must win")
    `SVTEST_CHECK(a_vld_events >= 3, "s13 vif/class partial+whole: downstream must see vld")
  endtask

  // s14 COUSIN: partial, whole, whole (triple, same timestep).
  task s14_partial_whole_whole;
    @(posedge clk) ci.bus_a.vld <= 1'b0;
                   ci.bus_a     <= PKT0;
                   ci.bus_a     <= PKT;
    @(negedge clk);
    dump_pkt("s14", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s14 partial+whole+whole: last whole-struct NBA must win")
  endtask

  // s15 COUSIN: whole, partial, whole (triple, same timestep).
  task s15_whole_partial_whole;
    @(posedge clk) ci.bus_a     <= PKT0;
                   ci.bus_a.vld <= 1'b0;
                   ci.bus_a     <= PKT;
    @(negedge clk);
    dump_pkt("s15", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT, "s15 whole+partial+whole: last whole-struct NBA must win")
  endtask

  // s16 CONTROL via the production data-beat path: class + vif, whole+whole.
  task s16_vif_class_beat_whole_whole;
    producer   drv;
    beat_info  bi;
    drv = new();
    drv.vif = ci;
    repeat (3) begin
      bi = new();
      bi.beat = BEAT;
      bi.place_delay = 0;
      drv.beat_q.put(bi);
      drv.beat_cr.put(1'b0);
    end
    sample_en = 2;
    drv.run_beat(3);
    repeat (2) @(posedge clk);   // let downstream samples of the last placements fire
    @(negedge clk);
    sample_en = 0;
    $display("SHAPE s16: bus_b.vld=%b data=%h strb=%h last=%b b_vld_events=%0d",
             ci.bus_b.vld, ci.bus_b.data, ci.bus_b.strb, ci.bus_b.last, b_vld_events);
    `SVTEST_CHECK(ci.bus_b == BEAT, "s16 vif/class whole+whole: final value should be BEAT")
    `SVTEST_CHECK(b_vld_events >= 3, "s16 vif/class whole+whole: downstream must see vld")
  endtask

  // ==================================================================
  // s17..s33 -- new target scopes and lvalue forms for the CORE shape:
  // a partial-region NBA followed, in the SAME timestep, by a
  // whole-variable NBA.  Reference simulator: the last (whole) NBA wins.
  // ==================================================================

  // s17 COUSIN: packed union -- member NBA then whole NBA.
  task s17_union_member_then_whole;
    @(posedge clk) uni.w <= 32'h0;
                   uni   <= UN;
    @(negedge clk);
    $display("SHAPE s17: uni=%h w=%h want=%h", uni, uni.w, UN);
    `SVTEST_CHECK(uni.w == UN, "s17 union-member+whole: whole NBA must win")
  endtask

  // s18 COUSIN: third level of nested packed structs.
  task s18_third_level_member_then_whole;
    @(posedge clk) n3.hdr2.vld <= 1'b0;
                   n3          <= NPKT3;
    @(negedge clk);
    $display("SHAPE s18: n3=%h hdr2.vld=%b hdr2.tag=%h", n3, n3.hdr2.vld, n3.hdr2.tag);
    `SVTEST_CHECK(n3 == NPKT3, "s18 3-level-member+whole: whole-struct NBA must win")
  endtask

  // s19 COUSIN: part-select OF A MEMBER, then whole struct.
  task s19_member_partsel_then_whole;
    @(posedge clk) q19.addr[7:0] <= 8'hAA;
                   q19           <= PKT;
    @(negedge clk);
    dump_pkt("s19", q19);
    `SVTEST_CHECK(q19 == PKT, "s19 member-partsel+whole: whole-struct NBA must win")
  endtask

  // s20 COUSIN: variable indexed part-select (+:), then whole vector.
  task s20_indexed_partsel_pos_then_whole;
    @(posedge clk) v20[sel*8 +: 8] <= 8'hAA;
                   v20             <= pkt_bits;
    @(negedge clk);
    $display("SHAPE s20: v20=%h want=%h", v20, pkt_bits);
    `SVTEST_CHECK(v20 == pkt_bits, "s20 indexed-partsel(+:)+whole: whole NBA must win")
  endtask

  // s21 COUSIN: variable indexed part-select (-:), then whole vector.
  task s21_indexed_partsel_neg_then_whole;
    @(posedge clk) v21[47 -: 8] <= 8'hAA;
                   v21          <= pkt_bits;
    @(negedge clk);
    $display("SHAPE s21: v21=%h want=%h", v21, pkt_bits);
    `SVTEST_CHECK(v21 == pkt_bits, "s21 indexed-partsel(-:)+whole: whole NBA must win")
  endtask

  // s22 COUSIN: concatenation lvalue, then whole vector.
  task s22_concat_lvalue_then_whole;
    @(posedge clk) {v22[59:56], v22[3:0]} <= 8'h00;
                   v22                    <= pkt_bits;
    @(negedge clk);
    $display("SHAPE s22: v22=%h want=%h", v22, pkt_bits);
    `SVTEST_CHECK(v22 == pkt_bits, "s22 concat-lvalue+whole: whole-vector NBA must win")
  endtask

  // s23 COUSIN: element of an interface ARRAY, member NBA then element whole.
  task s23_ifarray_elem_then_whole;
    @(posedge clk) cia[0].bus_a.vld <= 1'b0;
                   cia[0].bus_a      <= PKT;
    @(negedge clk);
    dump_pkt("s23", cia[0].bus_a);
    `SVTEST_CHECK(cia[0].bus_a == PKT, "s23 if-array-element+whole: whole NBA must win")
  endtask

  // s24 COUSIN: hierarchical reference into a submodule instance.
  task s24_hier_ref_then_whole;
    @(posedge clk) sub.mv.vld <= 1'b0;
                   sub.mv     <= PKT;
    @(negedge clk);
    dump_pkt("s24", sub.mv);
    `SVTEST_CHECK(sub.mv == PKT, "s24 hierarchical-ref+whole: whole NBA must win")
  endtask

  // s25 COUSIN: class-object property, member NBA then whole NBA.
  task s25_class_prop_member_then_whole;
    @(posedge clk) hdl.p.vld <= 1'b0;
                   hdl.p     <= PKT;
    @(negedge clk);
    dump_pkt("s25", hdl.p);
    `SVTEST_CHECK(hdl.p == PKT, "s25 class-prop member+whole: whole NBA must win")
  endtask

  // s26 COUSIN: member NBA then a whole-variable '0 fill.
  task s26_member_then_zero_fill;
    @(posedge clk) q26.vld <= 1'b1;
                   q26     <= '0;
    @(negedge clk);
    dump_pkt("s26", q26);
    `SVTEST_CHECK(q26 == '0, "s26 member+zero-fill: whole '0 NBA must win")
  endtask

  // s27 COUSIN: partial, whole, partial (triple, same timestep).
  task s27_partial_whole_partial;
    @(posedge clk) q27.vld <= 1'b0;
                   q27     <= PKT;
                   q27.len <= 8'h11;
    @(negedge clk);
    dump_pkt("s27", q27);
    `SVTEST_CHECK(q27 == PKT_L11, "s27 partial+whole+partial: whole must win for vld")
  endtask

  // s28 COUSIN: partial NBA with a #0 intra-assignment delay, then whole.
  task s28_delay0_partial_then_whole;
    @(posedge clk) q28.vld <= #0 1'b0;
                   q28     <= PKT;
    @(negedge clk);
    dump_pkt("s28", q28);
    `SVTEST_CHECK(q28 == PKT, "s28 #0-partial+whole: whole-struct NBA must win")
  endtask

  // s29 COUSIN: partial NBA issued from inside a function, then whole.
  task s29_function_partial_then_whole;
    @(posedge clk) fn29();
                   q29     <= PKT;
    @(negedge clk);
    dump_pkt("s29", q29);
    `SVTEST_CHECK(q29 == PKT, "s29 function-partial+whole: whole-struct NBA must win")
  endtask

  // s30 COUSIN: partial NBAs issued from a foreach, then element whole.
  task s30_foreach_partial_then_whole;
    @(posedge clk);
    foreach (arr30[i]) arr30[i].vld <= 1'b0;
    arr30[0] <= PKT;
    @(negedge clk);
    dump_pkt("s30", arr30[0]);
    `SVTEST_CHECK(arr30[0] == PKT, "s30 foreach-partial+whole: whole NBA must win")
  endtask

  // s31 COUSIN: member NBA then a whole assignment-pattern NBA.
  task s31_member_then_pattern_whole;
    @(posedge clk) q31.vld <= 1'b0;
                   q31 <= '{1'b1, 1'b1, 40'h0000_C0DE, 3'd2, 3'd2, 8'd0, 1'b0, 2'd1, 1'b0};
    @(negedge clk);
    dump_pkt("s31", q31);
    `SVTEST_CHECK(q31 == PKT, "s31 member+pattern-whole: whole NBA must win")
  endtask

  // s32 COUSIN: partial NBA inside fork/join, whole NBA right after.
  task s32_fork_partial_then_whole;
    @(posedge clk);
    fork
      q32.vld <= 1'b0;
    join
    q32 <= PKT;
    @(negedge clk);
    dump_pkt("s32", q32);
    `SVTEST_CHECK(q32 == PKT, "s32 fork-partial+whole: whole-struct NBA must win")
  endtask

  // s33 COUSIN: partial in delta 0, whole one delta later (#0).
  task s33_partial_delta0_whole_delta1;
    @(posedge clk) q33.vld <= 1'b0;
    #0 q33 <= PKT;
    @(negedge clk);
    dump_pkt("s33", q33);
    `SVTEST_CHECK(q33 == PKT, "s33 partial-d0+whole-d1: whole-struct NBA must win")
  endtask

  // ==================================================================
  // s34..s39 -- process / code-path axis.  The SAME core shape placed in
  // different processes and statement contexts.  On xezim the always-path
  // variants are correct while the initial-block variants are not, which
  // localises the bug to one of the two statement-execution paths.
  // ==================================================================

  // s34 CONTROL: identical core shape inside always @(posedge clk).
  task s34_always_path_control;
    en34 = 1; @(posedge clk); @(negedge clk); en34 = 0;
    dump_pkt("s34", q34);
    `SVTEST_CHECK(q34 == PKT, "s34 always-path control: whole-struct NBA must win")
  endtask

  // s35 CONTROL: identical core shape inside always_ff @(posedge clk).
  task s35_always_ff_control;
    en35 = 1; @(posedge clk); @(negedge clk); en35 = 0;
    dump_pkt("s35", q35);
    `SVTEST_CHECK(q35 == PKT, "s35 always_ff control: whole-struct NBA must win")
  endtask

  // s36 CONTROL: identical core shape inside initial forever @(posedge clk).
  task s36_initial_forever_control;
    en36 = 1; @(posedge clk); @(negedge clk); en36 = 0;
    dump_pkt("s36", q36);
    `SVTEST_CHECK(q36 == PKT, "s36 initial-forever control: whole-struct NBA must win")
  endtask

  // s37 COUSIN: identical core shape inside a for-loop body of an initial.
  task s37_for_loop_body;
    @(posedge clk);
    for (int i = 0; i < 1; i++) begin q37.vld <= 1'b0; q37 <= PKT; end
    @(negedge clk);
    dump_pkt("s37", q37);
    `SVTEST_CHECK(q37 == PKT, "s37 for-loop body: whole-struct NBA must win")
  endtask

  // s38 COUSIN: identical core shape inside a case arm of an initial.
  task s38_case_arm;
    @(posedge clk);
    case (1)
      1: begin q38.vld <= 1'b0; q38 <= PKT; end
    endcase
    @(negedge clk);
    dump_pkt("s38", q38);
    `SVTEST_CHECK(q38 == PKT, "s38 case-arm: whole-struct NBA must win")
  endtask

  // s39 COUSIN: identical core shape inside a while-loop body of an initial.
  task s39_while_body;
    @(posedge clk);
    begin
      int guard; guard = 0;
      while (guard == 0) begin
        q39.vld <= 1'b0;
        q39     <= PKT;
        guard = 1;
      end
    end
    @(negedge clk);
    dump_pkt("s39", q39);
    `SVTEST_CHECK(q39 == PKT, "s39 while-body: whole-struct NBA must win")
  endtask

  // ==================================================================
  // s40..s43 -- INDEPENDENT cousin bug: a whole-variable NBA whose target
  // is a CLASS-OBJECT PROPERTY silently keeps only the low 32 bits of the
  // RHS.  No partial NBA is involved; blocking assignment and module-scope
  // NBAs of the same type are both correct.
  // ==================================================================

  // s40 COUSIN: whole NBA of a 60-bit struct into a class property.
  task s40_class_prop_whole_nba_width;
    @(posedge clk) hdl2.p <= PKT;
    @(negedge clk);
    dump_pkt("s40", hdl2.p);
    `SVTEST_CHECK(hdl2.p == PKT,
                 "s40 class-prop whole NBA: RHS must not truncate to 32 bits")
  endtask

  // s41 COUSIN: whole NBA of a 64-bit vector into a class property.
  task s41_class_prop_vec64_nba_width;
    @(posedge clk) hdl2.vec <= VEC64;
    @(negedge clk);
    $display("SHAPE s41: hdl2.vec=%h want=%h", hdl2.vec, VEC64);
    `SVTEST_CHECK(hdl2.vec === VEC64,
                 "s41 class-prop 64b NBA: RHS must not truncate to 32 bits")
  endtask

  // s42 CONTROL: blocking assignment to the very same properties.
  task s42_class_prop_blocking_control;
    hdl2.p   = PKT;
    hdl2.vec = VEC64;
    @(negedge clk);
    dump_pkt("s42", hdl2.p);
    `SVTEST_CHECK(hdl2.p === PKT && hdl2.vec === VEC64,
                 "s42 class-prop blocking control: value must land in full")
  endtask

  // s43 CONTROL: whole NBA to a module variable of the same type.
  task s43_module_var_nba_width_control;
    @(posedge clk) mv43 <= PKT;
    @(negedge clk);
    dump_pkt("s43", mv43);
    `SVTEST_CHECK(mv43 == PKT, "s43 module-var NBA control: value must land in full")
  endtask

  // ==================================================================
  // s44..s46 -- Bug A SCOPE cousins: the SAME core shape (partial-member
  // NBA then same-timestep whole-struct NBA) with the target declared in
  // a different scope.  The ordering defect follows the target's scope,
  // not its type.
  // ==================================================================

  // s44 COUSIN: package-scope variable target.
  task s44_package_var_partial_then_whole;
    nba_types::pkg_pkt = PKT0;
    @(posedge clk) nba_types::pkg_pkt.vld <= 1'b0;
                   nba_types::pkg_pkt     <= PKT;
    @(negedge clk);
    dump_pkt("s44", nba_types::pkg_pkt);
    `SVTEST_CHECK(nba_types::pkg_pkt == PKT,
                 "s44 package-var partial+whole: whole-struct NBA must win")
  endtask

  // s45 COUSIN: generate-block variable target.
  task s45_generate_var_partial_then_whole;
    gscope.gen_pkt = PKT0;
    @(posedge clk) gscope.gen_pkt.vld <= 1'b0;
                   gscope.gen_pkt     <= PKT;
    @(negedge clk);
    dump_pkt("s45", gscope.gen_pkt);
    `SVTEST_CHECK(gscope.gen_pkt == PKT,
                 "s45 generate-var partial+whole: whole-struct NBA must win")
  endtask

  // s46 CONTROL: the same submodule variable as s24, but reached through a
  // $root-anchored hierarchical name instead of a relative one.
  task s46_root_hier_ref_then_whole;
    sub.mv = PKT0;
    @(posedge clk) $root.test_nba_struct_shapes.sub.mv.vld <= 1'b0;
                   $root.test_nba_struct_shapes.sub.mv     <= PKT;
    @(negedge clk);
    dump_pkt("s46", sub.mv);
    `SVTEST_CHECK(sub.mv == PKT,
                 "s46 $root-anchored hier ref: whole-struct NBA must win")
  endtask

  // ==================================================================
  // s47..s54 -- Bug B SCOPE cousins: the NBA target WIDTH of a class
  // property.  A HANDLE-qualified property (`h.prop`) is sized by the
  // 32-bit default, so a wider RHS is truncated and a `real` RHS is
  // converted to a 32-bit integer.  `this.`-qualified and bare-name
  // targets keep the declared width.
  // ==================================================================

  // s47 CONTROL: 32-bit handle-qualified property -- exactly at the default.
  task s47_prop_w32_control;
    wh.w32 = '0;
    @(posedge clk) wh.w32 <= 32'hFFFF_FFFF;
    @(negedge clk);
    $display("SHAPE s47: wh.w32=%h want=ffffffff", wh.w32);
    `SVTEST_CHECK(wh.w32 === 32'hFFFF_FFFF,
                 "s47 handle-qualified 32b property: value must land in full")
  endtask

  // s48 COUSIN: 33-bit handle-qualified property -- one bit over the default.
  task s48_prop_w33;
    wh.w33 = '0;
    @(posedge clk) wh.w33 <= 33'h1_FFFF_FFFF;
    @(negedge clk);
    $display("SHAPE s48: wh.w33=%h want=1ffffffff", wh.w33);
    `SVTEST_CHECK(wh.w33 === 33'h1_FFFF_FFFF,
                 "s48 handle-qualified 33b property: RHS must not truncate to 32 bits")
  endtask

  // s49 COUSIN: 64-bit handle-qualified property.
  task s49_prop_w64;
    wh.w64 = '0;
    @(posedge clk) wh.w64 <= 64'h0123_4567_89AB_CDEF;
    @(negedge clk);
    $display("SHAPE s49: wh.w64=%h want=0123456789abcdef", wh.w64);
    `SVTEST_CHECK(wh.w64 === 64'h0123_4567_89AB_CDEF,
                 "s49 handle-qualified 64b property: RHS must not truncate to 32 bits")
  endtask

  // s50 COUSIN: `real` handle-qualified property -- converted to int32.
  task s50_prop_real;
    wh.r = 0.0;
    @(posedge clk) wh.r <= -1.5;
    @(negedge clk);
    $display("SHAPE s50: wh.r=%f want=-1.500000", wh.r);
    `SVTEST_CHECK(wh.r == -1.5,
                 "s50 handle-qualified real property: NBA must keep the real value")
  endtask

  // s51 CONTROL: the same two properties via `this.` inside a method.
  task s51_this_qualified_control;
    wh.w64 = '0; wh.r = 0.0;
    @(posedge clk) wh.nba_this_wide();
    @(negedge clk);
    $display("SHAPE s51: wh.w64=%h wh.r=%f want=0123456789abcdef -1.500000",
             wh.w64, wh.r);
    `SVTEST_CHECK(wh.w64 === 64'h0123_4567_89AB_CDEF && wh.r == -1.5,
                 "s51 this.-qualified property NBA control: value must land in full")
  endtask

  // s52 COUSIN: bare-name `real` property inside a method (implicit this).
  task s52_bare_real_prop;
    wh.r = 0.0;
    @(posedge clk) wh.nba_bare_real();
    @(negedge clk);
    $display("SHAPE s52: wh.r=%f want=-1.500000", wh.r);
    `SVTEST_CHECK(wh.r == -1.5,
                 "s52 bare-name real property: NBA must keep the real value")
  endtask

  // s53 COUSIN: 64-bit NBA into a class STATIC property.
  task s53_static_prop_w64;
    static_holder::sv = '0;
    @(posedge clk) static_holder::sv <= 64'h0123_4567_89AB_CDEF;
    @(negedge clk);
    $display("SHAPE s53: static_holder::sv=%h want=0123456789abcdef",
             static_holder::sv);
    `SVTEST_CHECK(static_holder::sv === 64'h0123_4567_89AB_CDEF,
                 "s53 class-static 64b property: RHS must not truncate to 32 bits")
  endtask

  // s54 COUSIN: whole-struct NBA into a class STATIC property.  The core
  // shape truncates to 32 bits here too (vld/wr are above bit 31).
  task s54_static_prop_whole_nba;
    static_holder::sp = PKT0;
    @(posedge clk) static_holder::sp <= PKT;
    @(negedge clk);
    dump_pkt("s54", static_holder::sp);
    `SVTEST_CHECK(static_holder::sp == PKT,
                 "s54 class-static whole NBA: value must land in full")
  endtask

  // s61 CONTROL: the same whole-struct assignment, blocking.
  task s61_static_prop_whole_blocking_control;
    static_holder::sp = PKT0;
    static_holder::sp = PKT;
    dump_pkt("s61", static_holder::sp);
    `SVTEST_CHECK(static_holder::sp == PKT,
                 "s61 class-static whole blocking control: value must land in full")
  endtask

  // ==================================================================
  // s55..s57 -- Bug C: an NBA whose target is a `super.`-qualified
  // property never lands at all.  Blocking assignment to the same
  // property is correct, and `this.`-qualified NBA is correct.
  // ==================================================================

  // s55 COUSIN: whole NBA to a super.-qualified property.
  task s55_super_whole_nba;
    dh.bp = PKT0;
    @(posedge clk) dh.nba_super_whole(PKT);
    @(negedge clk);
    dump_pkt("s55", dh.bp);
    `SVTEST_CHECK(dh.bp == PKT, "s55 super.-qualified whole NBA: value must land")
  endtask

  // s56 CONTROL: blocking assignment to the same super.-qualified property.
  task s56_super_blocking_control;
    dh.bp = PKT0;
    dh.blk_super_whole(PKT);
    $display("SHAPE s56: dh.bp=%h want=%h", dh.bp, PKT);
    `SVTEST_CHECK(dh.bp == PKT,
                 "s56 super.-qualified blocking control: value must land")
  endtask

  // s57 COUSIN: the core partial+whole shape on a super.-qualified property.
  task s57_super_partial_then_whole;
    dh.bp = PKT0;
    @(posedge clk) dh.nba_super_partial_whole(PKT);
    @(negedge clk);
    dump_pkt("s57", dh.bp);
    `SVTEST_CHECK(dh.bp == PKT,
                 "s57 super.-qualified partial+whole: whole-struct NBA must win")
  endtask

  // ==================================================================
  // s58..s60 -- Related SCOPE defects (NOT NBA ordering): a modport-
  // qualified write reaches a phantom copy of the interface variable, and
  // a class-scope static property's member view is a different storage
  // from the whole property.  Included because they make the matching
  // NBA shapes fail for a reason other than ordering.
  // ==================================================================

  // s58 COUSIN: modport-qualified BLOCKING write (no NBA involved).  The
  // write lands in a phantom copy, so the interface variable keeps PKT0.
  task s58_modport_blocking_write;
    ci.bus_a = PKT0;
    ci.drv.bus_a = PKT;
    $display("SHAPE s58: ci.bus_a=%h ci.drv.bus_a=%h want=%h",
             ci.bus_a, ci.drv.bus_a, PKT);
    `SVTEST_CHECK(ci.bus_a == PKT,
                 "s58 modport-qualified blocking write: must reach the interface variable")
  endtask

  // s59 COUSIN: modport-qualified partial+whole NBA.
  task s59_modport_partial_then_whole;
    ci.bus_a = PKT0;
    @(posedge clk) ci.drv.bus_a.vld <= 1'b0;
                   ci.drv.bus_a     <= PKT;
    @(negedge clk);
    dump_pkt("s59", ci.bus_a);
    `SVTEST_CHECK(ci.bus_a == PKT,
                 "s59 modport partial+whole: whole-struct NBA must win")
  endtask

  // s60 COUSIN: class-scope static property member read after a whole write.
  task s60_static_prop_member_read;
    static_holder::sp = PKT;
    $display("SHAPE s60: static_holder::sp=%h sp.vld=%b sp.wr=%b want=%h 1 1",
             static_holder::sp, static_holder::sp.vld, static_holder::sp.wr, PKT);
    `SVTEST_CHECK(static_holder::sp.vld === 1'b1 && static_holder::sp.wr === 1'b1,
                 "s60 class-static member read: must see the whole-property write")
  endtask

  initial begin
    repeat (2) @(posedge clk);
    pkt_bits = {PKT.vld, PKT.wr, PKT.addr, PKT.tag, PKT.sz, PKT.len, PKT.attr, PKT.mode, PKT.hold};

    s01_whole_whole;
    s02_partial_vld_then_whole;
    s03_whole_then_partial_vld;
    s04_partial_addr_then_whole;
    s05_partial_multi_then_whole;
    s06_partial_then_whole_next_cycle;
    s07_vec_bit_then_whole;
    s08_vec_partsel_then_whole;
    s09_nested_member_then_whole;
    s10_module_var_partial_then_whole;
    s11_unpacked_partial_then_whole;
    s12_array_elem_partial_then_whole;
    s13_vif_class_partial_then_whole;
    s14_partial_whole_whole;
    s15_whole_partial_whole;
    s16_vif_class_beat_whole_whole;

    hdl  = new();
    hdl2 = new();
    foreach (arr30[i]) arr30[i] = PKT0;

    s17_union_member_then_whole;
    s18_third_level_member_then_whole;
    s19_member_partsel_then_whole;
    s20_indexed_partsel_pos_then_whole;
    s21_indexed_partsel_neg_then_whole;
    s22_concat_lvalue_then_whole;
    s23_ifarray_elem_then_whole;
    s24_hier_ref_then_whole;
    s25_class_prop_member_then_whole;
    s26_member_then_zero_fill;
    s27_partial_whole_partial;
    s28_delay0_partial_then_whole;
    s29_function_partial_then_whole;
    s30_foreach_partial_then_whole;
    s31_member_then_pattern_whole;
    s32_fork_partial_then_whole;
    s33_partial_delta0_whole_delta1;
    s34_always_path_control;
    s35_always_ff_control;
    s36_initial_forever_control;
    s37_for_loop_body;
    s38_case_arm;
    s39_while_body;
    s40_class_prop_whole_nba_width;
    s41_class_prop_vec64_nba_width;
    s42_class_prop_blocking_control;
    s43_module_var_nba_width_control;

    wh = new();
    dh = new();

    s44_package_var_partial_then_whole;
    s45_generate_var_partial_then_whole;
    s46_root_hier_ref_then_whole;
    s47_prop_w32_control;
    s48_prop_w33;
    s49_prop_w64;
    s50_prop_real;
    s51_this_qualified_control;
    s52_bare_real_prop;
    s53_static_prop_w64;
    s54_static_prop_whole_nba;
    s61_static_prop_whole_blocking_control;
    s55_super_whole_nba;
    s56_super_blocking_control;
    s57_super_partial_then_whole;
    s58_modport_blocking_write;
    s59_modport_partial_then_whole;
    s60_static_prop_member_read;

    `SVTEST_PASSFAIL
    $finish;
  end

endmodule
