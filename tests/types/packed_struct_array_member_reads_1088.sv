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

// -----------------------------------------------------------------------------
// Shared types (file scope so every module below sees them).
// -----------------------------------------------------------------------------
typedef logic [9:0] sample_t; // one lane sample

// One entry of a small candidate list carried on a wide config bus.
typedef struct packed {
  logic       valid;
  logic [5:0] dir;  // direction code
  logic [3:0] mode; // mode code
} hdr_t;

// Wide config bus: a packed array of candidate entries plus a plain member.
typedef struct packed {
  hdr_t [0:2]  slots;
  logic [9:0]  base;
} bus_pkt_t;

// Beat tag: the low bits of `way` select one of bus_pkt_t.slots[0:2].
typedef struct packed {
  logic [1:0] unit;
  logic [6:0] seq;
  logic [2:0] way;
} tag_t;

// Reference snapshot queued by the monitor: an unpacked struct whose `pix`
// field is a packed array of lane samples.
typedef struct {
  tag_t tag_out;
  tag_t tag_ev;
  tag_t tag_od;
  sample_t [0:15] pix;
} snap_t;

// -----------------------------------------------------------------------------
// Scope-probe types: the same packed array of packed structs declared with
// different ranges / nesting, to pin down which shapes are mis-read.
// -----------------------------------------------------------------------------
// Descending range: element 0 is the LSB element.
typedef struct packed {
  hdr_t [2:0] slots;
  logic [9:0] base;
} desc_pkt_t;

// Ascending range, N = 2.
typedef struct packed {
  hdr_t [0:1] slots;
  logic [9:0] base;
} asc2_pkt_t;

// Ascending range, N = 4.
typedef struct packed {
  hdr_t [0:3] slots;
  logic [9:0] base;
} asc4_pkt_t;

// Ascending range with a non-zero left bound: there is no `slots[0]` element.
typedef struct packed {
  hdr_t [1:3] slots;
  logic [9:0] base;
} asc13_pkt_t;

// Two-dimensional packed array of packed structs.
typedef struct packed {
  hdr_t [0:1][0:1] grid;
  logic [9:0] base;
} grid_pkt_t;

// Two-dimensional packed array of packed structs, DESCENDING in both dims:
// control for the 2-D shape (a descending range labels element 0 as the LSB
// element, so the element-order question does not arise).
typedef struct packed {
  hdr_t [1:0][1:0] grid;
  logic [9:0] base;
} gridd_pkt_t;

// -----------------------------------------------------------------------------
// DUT: picks a candidate entry off the config bus with continuous assigns.
//
// Mirrors a mode-select front end: the array index is a part select of a
// struct member (tag_i.way[1:0]), plus constant-index and plain-variable
// index cousins, plus a plain struct member as control.
// -----------------------------------------------------------------------------
module agg_read_dut (
  input  bus_pkt_t   bus_i,
  input  tag_t       tag_i,
  input  logic [1:0] sel_i,
  output logic [5:0] dir_member_o,  // index: struct-member part select
  output logic [3:0] mode_member_o, // index: struct-member part select
  output logic [5:0] dir_const_o,   // index: constant
  output logic [5:0] dir_var_o,     // index: plain variable
  output logic [9:0] base_o         // control: plain struct member
);

  assign dir_member_o  = bus_i.slots[tag_i.way[1:0]].dir;
  assign mode_member_o = bus_i.slots[tag_i.way[1:0]].mode;
  assign dir_const_o   = bus_i.slots[2'd0].dir;
  assign dir_var_o     = bus_i.slots[sel_i].dir;
  assign base_o        = bus_i.base;

endmodule

// -----------------------------------------------------------------------------
// Monitor: queues expected snapshots, pops them on output beats and compares
// lane samples against the observed beat.
//
// Mirrors a scoreboard: queue of unpacked structs, pop_front into a module
// variable, and an automatic function that reads individual elements of the
// packed-array field (`pix`) of that struct.
// -----------------------------------------------------------------------------
module snap_check_mon (
  input logic                clk,
  input logic                push_i,
  input logic                pop_i,
  input sample_t [0:15]      obs_i,
  output int                 fails_o
);

  `SVTEST_INIT

  snap_t                exp_q[$];
  snap_t                exp_snap;
  sample_t [0:15]       whole_copy; // whole-field copy of exp_snap.pix
  logic [159:0]         pix_flat;   // flat view of exp_snap.pix, for bit-position part-selects
  logic [9:0]           ps_read;    // part-select read of one element
  logic [9:0]           const_read; // constant-index read of one element
  bit                   cousins_checked;

  // Compare every lane of the observed beat against the popped snapshot.
  // `whole_copy` holds the same data through a whole-field read, so it shows
  // what is actually stored while `exp_snap.pix[i]` shows what an indexed
  // element read returns.
  function automatic void check_beat();
    for (int i = 0; i < 16; i++) begin
      if (obs_i[i] !== exp_snap.pix[i]) begin
        failures++;
        $display("FAIL: lane %0d observed %0d, indexed read of snapshot gives %0d (stored value is %0d)",
                 i, obs_i[i], exp_snap.pix[i], whole_copy[i]);
      end
    end
  endfunction

  always @(posedge clk) begin
    if (push_i) begin
      snap_t ent;
      for (int li = 0; li < 16; li++) begin
        ent.pix[li] = sample_t'(100 + li);
      end
      ent.tag_out = '{unit: 2'd2, seq: 7'd0, way: 3'd4};
      ent.tag_ev  = ent.tag_out;
      ent.tag_od  = ent.tag_out;
      exp_q.push_back(ent);
    end
    if (pop_i) begin
      exp_snap   = exp_q.pop_front();
      whole_copy = exp_snap.pix;            // whole-field read
      // `pix` is a PACKED sample_t [0:15] with ASCENDING bounds, so element 0
      // sits at the MSB end and element i occupies bits [159-10i -: 10];
      // element 3 is bits [129:120]. A part-select written directly on `pix`
      // indexes the DECLARED element range 0:15, not flat bit positions, so
      // `pix[120 +: 10]` is out of bounds and reads x. Take the 10-bit window
      // by bit position from a flat view of the field instead.
      pix_flat   = exp_snap.pix;
      ps_read    = pix_flat[120 +: 10];     // part-select element read
      const_read = exp_snap.pix[3];         // constant-index element read
      check_beat();
      if (!cousins_checked) begin
        cousins_checked = 1'b1;
        `SVTEST_CHECK(whole_copy[3] === 10'd103, "whole-field copy of packed-array field")
        `SVTEST_CHECK(ps_read === 10'd103, "part-select read of packed-array field element")
        `SVTEST_CHECK(const_read === 10'd103, "constant-index read of packed-array field element")
      end
    end
  end

  assign fails_o = failures;

endmodule

// -----------------------------------------------------------------------------
// Scope probe: pins down exactly which shapes of
// `container.packed_array_of_struct[i].leaf` are mis-read.
//
// Every read below is a continuous assign, i.e. the compiled path. The very
// same expressions read procedurally (in tb_top) return the correct values, so
// the stored data is always correct and only the compiled read path is wrong.
//
// The containers arrive as input ports, which is the shape that reproduces the
// original failure deterministically (a packed struct with an ascending
// array-of-struct member crossing a module boundary).
// -----------------------------------------------------------------------------
module packed_member_scope_probe (
  input  bus_pkt_t    asc,   // hdr_t [0:2] slots  ascending, N=3  (failing shape)
  input  desc_pkt_t   desc,  // hdr_t [2:0] slots  descending, N=3
  input  asc2_pkt_t   n2,    // hdr_t [0:1] slots  ascending, N=2
  input  asc4_pkt_t   n4,    // hdr_t [0:3] slots  ascending, N=4
  input  asc13_pkt_t  n13,   // hdr_t [1:3] slots  ascending, non-zero left bound
  input  grid_pkt_t   grid,  // hdr_t [0:1][0:1] grid  2-D packed array of structs
  input  hdr_t [0:2]  arr,   // packed array of structs as a whole signal
  input  logic [1:0]  idx,
  output int          fails_o
);

  `SVTEST_INIT

  // ---- compiled reads (continuous assigns) --------------------------------
  logic [5:0] asc0, asc1, asc2, asc_var;
  logic [3:0] asc_mode0, asc_mode1, asc_mode2;
  logic [3:0] asc_ps;
  hdr_t       asc_elem1;
  logic [9:0] asc_base;

  logic [5:0] desc0, desc1, desc2;

  logic [5:0] n2_0, n2_1;
  logic [5:0] n4_0, n4_1, n4_2, n4_3;
  logic [5:0] n13_1, n13_2, n13_3;

  logic [5:0] grid00, grid01, grid10, grid11;

  logic [5:0] arr0, arr1, arr2;

  // Ascending member range [0:2] -- the failing shape.
  assign asc0      = asc.slots[2'd0].dir;
  assign asc1      = asc.slots[2'd1].dir;
  assign asc2      = asc.slots[2'd2].dir;
  assign asc_var   = asc.slots[idx].dir;
  assign asc_mode0 = asc.slots[2'd0].mode;   // LSB field of the element
  assign asc_mode1 = asc.slots[2'd1].mode;
  assign asc_mode2 = asc.slots[2'd2].mode;
  assign asc_ps    = asc.slots[2'd1].dir[3:0]; // part-select of the member
  assign asc_elem1 = asc.slots[2'd1];          // whole element (control)
  assign asc_base  = asc.base;                 // plain member (control)

  // Descending member range [2:0] -- control.
  assign desc0 = desc.slots[2'd0].dir;
  assign desc1 = desc.slots[2'd1].dir;
  assign desc2 = desc.slots[2'd2].dir;

  // Ascending member range, other sizes.
  assign n2_0 = n2.slots[2'd0].dir;
  assign n2_1 = n2.slots[2'd1].dir;
  assign n4_0 = n4.slots[2'd0].dir;
  assign n4_1 = n4.slots[2'd1].dir;
  assign n4_2 = n4.slots[2'd2].dir;
  assign n4_3 = n4.slots[2'd3].dir;
  assign n13_1 = n13.slots[2'd1].dir;
  assign n13_2 = n13.slots[2'd2].dir;
  assign n13_3 = n13.slots[2'd3].dir;

  // Two-dimensional packed array of structs -- control.
  assign grid00 = grid.grid[2'd0][2'd0].dir;
  assign grid01 = grid.grid[2'd0][2'd1].dir;
  assign grid10 = grid.grid[2'd1][2'd0].dir;
  assign grid11 = grid.grid[2'd1][2'd1].dir;

  // Packed array of structs that is a whole signal, not a struct member.
  assign arr0 = arr[2'd0].dir;
  assign arr1 = arr[2'd1].dir;
  assign arr2 = arr[2'd2].dir;

  initial begin
    #1; // let the continuous assigns settle

    // ---- observed values (compiled path) ----------------------------------
    $display("SCOPE asc  [0:2] .dir  : %0d %0d %0d  (exp 39 56 46)", asc0, asc1, asc2);
    $display("SCOPE asc  [0:2] .dir v: %0d  (exp 56, idx==1)", asc_var);
    $display("SCOPE asc  [0:2] .mode : %0d %0d %0d  (exp 7 11 5)", asc_mode0, asc_mode1, asc_mode2);
    $display("SCOPE asc  [0:2] ps   : %0d  (exp 8)", asc_ps);
    $display("SCOPE asc  [0:2] elem : %0d  (exp 56)", asc_elem1.dir);
    $display("SCOPE asc  [0:2] base : %0d  (exp 475)", asc_base);
    $display("SCOPE desc [2:0] .dir  : %0d %0d %0d  (exp 39 56 46)", desc0, desc1, desc2);
    $display("SCOPE arr  whole .dir  : %0d %0d %0d  (exp 39 56 46)", arr0, arr1, arr2);
    $display("SCOPE grid [0:1][0:1]  : %0d %0d %0d %0d  (exp 11 22 33 44)", grid00, grid01, grid10, grid11);
    $display("SCOPE asc  [1:3] .dir  : %0d %0d %0d  (exp 11 22 33)", n13_1, n13_2, n13_3);
    $display("SCOPE asc  [0:1] .dir  : %0d %0d  (exp 11 22)", n2_0, n2_1);
    $display("SCOPE asc  [0:3] .dir  : %0d %0d %0d %0d  (exp 11 22 33 44)", n4_0, n4_1, n4_2, n4_3);

    // ---- controls: shapes that must keep working --------------------------
    `SVTEST_CHECK(asc_base === 10'd475, "scope: plain struct member through continuous assign")
    `SVTEST_CHECK(asc_elem1.dir === 6'd56, "scope: whole-element read asc.slots[1] (compiled)")
    `SVTEST_CHECK(desc0 === 6'd39 && desc1 === 6'd56 && desc2 === 6'd46,
                  "scope: descending member range [2:0] (compiled)")
    `SVTEST_CHECK(arr0 === 6'd39 && arr1 === 6'd56 && arr2 === 6'd46,
                  "scope: packed array of structs as a whole signal (compiled)")
    `SVTEST_CHECK(grid00 === 6'd11 && grid01 === 6'd22 && grid10 === 6'd33 && grid11 === 6'd44,
                  "scope: 2-D packed array of structs member (compiled)")
    `SVTEST_CHECK(n13_1 === 6'd11 && n13_2 === 6'd22 && n13_3 === 6'd33,
                  "scope: ascending member range with non-zero left bound [1:3] (compiled)")

    // ---- the failing shape: ascending member range [0:N-1] ----------------
    `SVTEST_CHECK(asc0 === 6'd39, "scope: asc.slots[0].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc1 === 6'd56, "scope: asc.slots[1].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc2 === 6'd46, "scope: asc.slots[2].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc_var === 6'd56, "scope: asc.slots[idx].dir with idx==1 (compiled)")
    `SVTEST_CHECK(asc_mode0 === 4'd7,  "scope: asc.slots[0].mode, LSB field of element (compiled)")
    `SVTEST_CHECK(asc_mode1 === 4'd11, "scope: asc.slots[1].mode, LSB field of element (compiled)")
    `SVTEST_CHECK(asc_mode2 === 4'd5,  "scope: asc.slots[2].mode, LSB field of element (compiled)")
    `SVTEST_CHECK(asc_ps === 4'd8, "scope: asc.slots[1].dir[3:0] part-select of member (compiled)")

    // Same shape at other sizes: the displacement grows with N.
    `SVTEST_CHECK(n2_0 === 6'd11 && n2_1 === 6'd22,
                  "scope: ascending member range [0:1], N=2 (compiled)")
    `SVTEST_CHECK(n4_0 === 6'd11 && n4_1 === 6'd22 && n4_2 === 6'd33 && n4_3 === 6'd44,
                  "scope: ascending member range [0:3], N=4 (compiled)")

    fails_o = failures;
  end

endmodule

// -----------------------------------------------------------------------------
// Context probe: exactly the reads of `packed_member_scope_probe`, but performed
// on a *local* aggregate that is continuously assigned from the port.
//
// The layout registration for an aggregate type is not consistent across
// declaration contexts. Reading the port directly (what `packed_member_scope_probe`
// does) and reading a local copy of that same port give different answers, and
// neither matches the reference. This module pins that down. It is a
// second-order finding: it explains why the reproduction is sensitive to how the
// container is declared, but it is not the cause of the original failure.
// -----------------------------------------------------------------------------
module packed_member_copy_probe (
  input  bus_pkt_t    asc,
  input  desc_pkt_t   desc,
  input  asc2_pkt_t   n2,
  input  asc4_pkt_t   n4,
  input  asc13_pkt_t  n13,
  input  grid_pkt_t   grid,
  input  logic [1:0]  idx,
  output int          fails_o
);

  `SVTEST_INIT

  // Local copies of the ports. A continuous assign into a local aggregate is
  // what changes the registration.
  bus_pkt_t   c_asc;
  desc_pkt_t  c_desc;
  asc2_pkt_t  c_n2;
  asc4_pkt_t  c_n4;
  asc13_pkt_t c_n13;
  grid_pkt_t  c_grid;

  assign c_asc  = asc;
  assign c_desc = desc;
  assign c_n2   = n2;
  assign c_n4   = n4;
  assign c_n13  = n13;
  assign c_grid = grid;

  logic [5:0] asc0, asc1, asc2, asc_var;
  logic [5:0] desc0, desc1, desc2;
  logic [5:0] n2_0, n2_1;
  logic [5:0] n4_0, n4_1, n4_2, n4_3;
  logic [5:0] n13_1, n13_2, n13_3;
  logic [5:0] grid00, grid01, grid10, grid11;

  assign asc0    = c_asc.slots[2'd0].dir;
  assign asc1    = c_asc.slots[2'd1].dir;
  assign asc2    = c_asc.slots[2'd2].dir;
  assign asc_var = c_asc.slots[idx].dir;

  assign desc0 = c_desc.slots[2'd0].dir;
  assign desc1 = c_desc.slots[2'd1].dir;
  assign desc2 = c_desc.slots[2'd2].dir;

  assign n2_0 = c_n2.slots[2'd0].dir;
  assign n2_1 = c_n2.slots[2'd1].dir;
  assign n4_0 = c_n4.slots[2'd0].dir;
  assign n4_1 = c_n4.slots[2'd1].dir;
  assign n4_2 = c_n4.slots[2'd2].dir;
  assign n4_3 = c_n4.slots[2'd3].dir;
  assign n13_1 = c_n13.slots[2'd1].dir;
  assign n13_2 = c_n13.slots[2'd2].dir;
  assign n13_3 = c_n13.slots[2'd3].dir;

  assign grid00 = c_grid.grid[2'd0][2'd0].dir;
  assign grid01 = c_grid.grid[2'd0][2'd1].dir;
  assign grid10 = c_grid.grid[2'd1][2'd0].dir;
  assign grid11 = c_grid.grid[2'd1][2'd1].dir;

  initial begin
    #1; // let the continuous assigns settle

    $display("COPY asc  [0:2] .dir  : %0d %0d %0d  (exp 39 56 46)", asc0, asc1, asc2);
    $display("COPY asc  [0:2] .dir v: %0d  (exp 39, idx==0)", asc_var);
    $display("COPY desc [2:0] .dir  : %0d %0d %0d  (exp 39 56 46)", desc0, desc1, desc2);
    $display("COPY asc  [0:1] .dir  : %0d %0d  (exp 11 22)", n2_0, n2_1);
    $display("COPY asc  [0:3] .dir  : %0d %0d %0d %0d  (exp 11 22 33 44)", n4_0, n4_1, n4_2, n4_3);
    $display("COPY asc  [1:3] .dir  : %0d %0d %0d  (exp 11 22 33)", n13_1, n13_2, n13_3);
    $display("COPY grid [0:1][0:1]  : %0d %0d %0d %0d  (exp 11 22 33 44)", grid00, grid01, grid10, grid11);

    // Descending range is the one shape that stays correct in this context.
    `SVTEST_CHECK(desc0 === 6'd39 && desc1 === 6'd56 && desc2 === 6'd46,
                  "copy: descending member range [2:0] (compiled)")

    // Ascending ranges: the element order comes back reversed, so index i
    // returns element (left+right)-i instead of element i.
    `SVTEST_CHECK(asc0 === 6'd39, "copy: asc.slots[0].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc1 === 6'd56, "copy: asc.slots[1].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc2 === 6'd46, "copy: asc.slots[2].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(asc_var === 6'd39, "copy: asc.slots[idx].dir with idx==0 (compiled)")
    `SVTEST_CHECK(n2_0 === 6'd11 && n2_1 === 6'd22,
                  "copy: ascending member range [0:1], N=2 (compiled)")
    `SVTEST_CHECK(n4_0 === 6'd11 && n4_1 === 6'd22 && n4_2 === 6'd33 && n4_3 === 6'd44,
                  "copy: ascending member range [0:3], N=4 (compiled)")
    `SVTEST_CHECK(n13_1 === 6'd11 && n13_2 === 6'd22 && n13_3 === 6'd33,
                  "copy: ascending member range [1:3] (compiled)")
    `SVTEST_CHECK(grid00 === 6'd11 && grid01 === 6'd22 && grid10 === 6'd33 && grid11 === 6'd44,
                  "copy: 2-D packed array of structs member (compiled)")

    fails_o = failures;
  end

endmodule

// -----------------------------------------------------------------------------
// Bug 3: procedural stores THROUGH a packed-array member of a packed struct
// (`c.slots[i].leaf = v`) inside an instantiated module.
//
// `st_leaf` writes one leaf of each element; `st_whole` writes the whole
// element. The two containers differ ONLY in the store form, and both are read
// back by byte-identical continuous-assign member reads. Inside an instantiated
// module the whole-element stores land and the leaf stores are silently DROPPED
// - the container reads back as if nothing had been written, and the whole
// variable shows only the `base` member.
//
// Run the same module as the designated top
// (`xezim packed_agg_elem_read.sv -s packed_store_probe`) and the picture
// inverts: the whole-variable read shows `270700b8005`, i.e. the leaf stores
// DID land, while the compiled member reads now return `x x 39` - the original
// bug of this MWE. Store and read are therefore broken in OPPOSITE contexts,
// which is why neither context ever shows a fully working path.
//
// Within this context the store is the broken side, not the read: `st_whole`
// proves the read path used by `lf0..lf2` returns correct values for data that
// did land, and the whole-variable read of `st_leaf` agrees that nothing is
// there to read.
// -----------------------------------------------------------------------------
module packed_store_probe (
  output int fails_o
);
  `SVTEST_INIT

  bus_pkt_t st_leaf;
  bus_pkt_t st_whole;

  logic [5:0] lf0, lf1, lf2;
  logic [5:0] wh0, wh1, wh2;
  assign lf0 = st_leaf.slots[2'd0].dir;
  assign lf1 = st_leaf.slots[2'd1].dir;
  assign lf2 = st_leaf.slots[2'd2].dir;
  assign wh0 = st_whole.slots[2'd0].dir;
  assign wh1 = st_whole.slots[2'd1].dir;
  assign wh2 = st_whole.slots[2'd2].dir;

  initial begin

    st_leaf = '0;
    st_leaf.slots[2'd0].dir = 6'd39;
    st_leaf.slots[2'd1].dir = 6'd56;
    st_leaf.slots[2'd2].dir = 6'd46;
    st_leaf.base            = 10'd5;

    st_whole = '0;
    st_whole.slots[0] = '{valid: 1'b1, dir: 6'd39, mode: 4'd7};
    st_whole.slots[1] = '{valid: 1'b0, dir: 6'd56, mode: 4'd11};
    st_whole.slots[2] = '{valid: 1'b1, dir: 6'd46, mode: 4'd5};
    st_whole.base     = 10'd475;

    #1;
    $display("STORE leaf  .dir  : %0d %0d %0d  (exp 39 56 46)", lf0, lf1, lf2);
    $display("STORE whole .dir  : %0d %0d %0d  (exp 39 56 46)", wh0, wh1, wh2);
    $display("STORE leaf  whole : %h  (exp 270700b8005)", st_leaf);

    `SVTEST_CHECK(lf0 === 6'd39, "store: st_leaf.slots[0].dir after a leaf store")
    `SVTEST_CHECK(lf1 === 6'd56, "store: st_leaf.slots[1].dir after a leaf store")
    `SVTEST_CHECK(lf2 === 6'd46, "store: st_leaf.slots[2].dir after a leaf store")
    `SVTEST_CHECK(wh0 === 6'd39, "store: st_whole.slots[0].dir after a whole store")
    `SVTEST_CHECK(wh1 === 6'd56, "store: st_whole.slots[1].dir after a whole store")
    `SVTEST_CHECK(wh2 === 6'd46, "store: st_whole.slots[2].dir after a whole store")
    // Independent read path (whole-variable) agrees with the member reads: the
    // leaf stores really are absent, not merely mis-read. Every slot of
    // st_leaf is zero, so the element-order issue of Bug 4 cannot explain this.
    `SVTEST_CHECK(st_leaf === 43'h270700B8005,
                  "store: whole-variable read of the leaf-stored container")

    fails_o = failures;
  end

endmodule

// -----------------------------------------------------------------------------
// Bug 4: reading a packed struct that CONTAINS an ascending packed array of
// packed structs as a WHOLE value (`assign w = c;`) inside an instantiated
// module reverses the element order.
//
// The container here is filled only with the whole-element store form that
// Bug 3 shows to be reliable, so the data in the variable is correct; only the
// whole-variable read is wrong. The member reads on the same container
// (`el0..el2`) are the control and read correctly.
//
// The value that comes back, `6e57179dddb`, is exactly the expected
// `677717b95db` with the three 11-bit elements in the opposite order, i.e.
// index i is being taken as (left+right)-i.
//
// Run this module as the designated top
// (`xezim packed_agg_elem_read.sv -s packed_whole_read_probe`) and the whole
// read is correct while the member reads break - the same opposite-context
// inversion as Bug 3.
// -----------------------------------------------------------------------------
module packed_whole_read_probe (
  output int fails_o
);
  `SVTEST_INIT

  bus_pkt_t wc;
  logic [42:0] w;
  logic [5:0]  el0, el1, el2;
  assign w   = wc;
  assign el0 = wc.slots[2'd0].dir;
  assign el1 = wc.slots[2'd1].dir;
  assign el2 = wc.slots[2'd2].dir;

  initial begin

    wc = '0;
    wc.slots[0] = '{valid: 1'b1, dir: 6'd39, mode: 4'd7};
    wc.slots[1] = '{valid: 1'b0, dir: 6'd56, mode: 4'd11};
    wc.slots[2] = '{valid: 1'b1, dir: 6'd46, mode: 4'd5};
    wc.base     = 10'd475;
    #1;

    $display("WHOLE member reads : %0d %0d %0d  (exp 39 56 46)", el0, el1, el2);
    $display("WHOLE assign w = c : %h  (exp 677717b95db)", w);

    `SVTEST_CHECK(el0 === 6'd39, "whole: member read slots[0].dir (control)")
    `SVTEST_CHECK(el1 === 6'd56, "whole: member read slots[1].dir (control)")
    `SVTEST_CHECK(el2 === 6'd46, "whole: member read slots[2].dir (control)")
    `SVTEST_CHECK(w === 43'h677717b95db,
                 "whole: whole-variable read of an ascending array-of-struct member")

    fails_o = failures;
  end

endmodule

// -----------------------------------------------------------------------------
// Bug 5: reading a member of a member that is a TWO-dimensional packed array of
// packed structs (`g.grid[i][j].leaf`) inside an instantiated module.
//
// The container is loaded with a single whole-variable assignment of the exact
// 54-bit pattern, so the data provably IS in the variable - the `gw = gd`
// whole-variable read below returns that pattern unchanged. Yet every
// `gd.grid[i][j].dir` member read returns 0. The DESCENDING container `gdd`
// (`[1:0][1:0]`, loaded with the equivalent pattern laid out for a descending
// range) reads 0 as well, so this is not the ascending element-order issue: the
// 2-D member select simply does not resolve at all.
//
// Run this same module as the designated top
// (`xezim packed_agg_elem_read.sv -s packed_grid2d_probe`) and all eight member
// reads return the right values.
//
// Note the original `packed_member_copy_probe` grid check reads 0 for the same
// reason; this probe removes the continuously-assigned local copy from the
// picture, so the 2-D member select itself is shown to be the broken side.
// -----------------------------------------------------------------------------
module packed_grid2d_probe (
  output int fails_o
);
  `SVTEST_INIT

  grid_pkt_t  gd;
  gridd_pkt_t gdd;

  logic [53:0] gw, gwd;
  logic [5:0] a00, a01, a10, a11;
  logic [5:0] d00, d01, d10, d11;
  assign gw  = gd;
  assign gwd = gdd;
  assign a00 = gd.grid[2'd0][2'd0].dir;
  assign a01 = gd.grid[2'd0][2'd1].dir;
  assign a10 = gd.grid[2'd1][2'd0].dir;
  assign a11 = gd.grid[2'd1][2'd1].dir;
  assign d00 = gdd.grid[2'd0][2'd0].dir;
  assign d01 = gdd.grid[2'd0][2'd1].dir;
  assign d10 = gdd.grid[2'd1][2'd0].dir;
  assign d11 = gdd.grid[2'd1][2'd1].dir;

  initial begin

    // grid[0][0] = {1,11,1} grid[0][1] = {0,22,2}
    // grid[1][0] = {1,33,3} grid[1][1] = {0,44,4} base = 0, element 0 at the
    // MSB end for the ascending range and at the LSB end for the descending one.
    gd  = 54'h258962C26B1000;
    gdd = 54'h1626132C52C400;
    #1;

    $display("GRID2D whole-variable read asc : %h  (exp 258962c26b1000)", gw);
    $display("GRID2D whole-variable read desc: %h  (exp 1626132c52c400)", gwd);
    $display("GRID2D asc  [0:1][0:1] .dir: %0d %0d %0d %0d  (exp 11 22 33 44)",
             a00, a01, a10, a11);
    $display("GRID2D desc [1:0][1:0] .dir: %0d %0d %0d %0d  (exp 11 22 33 44)",
             d00, d01, d10, d11);

    `SVTEST_CHECK(gw === 54'h258962C26B1000,
                  "grid2d: whole-variable read proves the data is stored")
    `SVTEST_CHECK(gwd === 54'h1626132C52C400,
                  "grid2d: whole-variable read proves the descending data is stored")
    `SVTEST_CHECK(a00 === 6'd11 && a01 === 6'd22 && a10 === 6'd33 && a11 === 6'd44,
                  "grid2d: ascending [0:1][0:1] member reads")
    `SVTEST_CHECK(d00 === 6'd11 && d01 === 6'd22 && d10 === 6'd33 && d11 === 6'd44,
                  "grid2d: descending [1:0][1:0] member reads")

    fails_o = failures;
  end

endmodule

// -----------------------------------------------------------------------------
// Verification Testbench
// -----------------------------------------------------------------------------
module tb_top;

  logic clk = 0;
  always #5 clk = ~clk;

  bus_pkt_t         bus;
  tag_t             tag;
  logic [1:0]       sel;
  logic [5:0]       dir_member, dir_const, dir_var;
  logic [3:0]       mode_member;
  logic [9:0]       base_out;
  sample_t [0:15]   obs;
  logic             push_beat = 0, pop_beat = 0;
  int               mon_fails;
  int               scope_fails;
  int               copy_fails;
  int               store_fails;
  int               whole_fails;
  int               grid2d_fails;

  // Containers for the scope probe (driven below, read inside the probe).
  bus_pkt_t         sc_asc;
  desc_pkt_t        sc_desc;
  asc2_pkt_t        sc_n2;
  asc4_pkt_t        sc_n4;
  asc13_pkt_t       sc_n13;
  grid_pkt_t        sc_grid;
  hdr_t [0:2]       sc_arr;
  logic [1:0]       sc_idx;
  logic [1:0]       cp_idx;

  // Context A: the same reads on a top-module variable rather than a port.
  bus_pkt_t         ctx_top;
  logic [1:0]       ctx_idx;
  logic [5:0]       top0, top1, top2, topv;
  assign top0 = ctx_top.slots[2'd0].dir;
  assign top1 = ctx_top.slots[2'd1].dir;
  assign top2 = ctx_top.slots[2'd2].dir;
  assign topv = ctx_top.slots[ctx_idx].dir;

  agg_read_dut u_dut (
    .bus_i        (bus),
    .tag_i        (tag),
    .sel_i        (sel),
    .dir_member_o (dir_member),
    .mode_member_o(mode_member),
    .dir_const_o  (dir_const),
    .dir_var_o    (dir_var),
    .base_o       (base_out)
  );

  snap_check_mon u_mon (
    .clk    (clk),
    .push_i (push_beat),
    .pop_i  (pop_beat),
    .obs_i  (obs),
    .fails_o(mon_fails)
  );

  packed_member_scope_probe u_scope (
    .asc    (sc_asc),
    .desc   (sc_desc),
    .n2     (sc_n2),
    .n4     (sc_n4),
    .n13    (sc_n13),
    .grid   (sc_grid),
    .arr    (sc_arr),
    .idx    (sc_idx),
    .fails_o(scope_fails)
  );

  packed_member_copy_probe u_copy (
    .asc    (sc_asc),
    .desc   (sc_desc),
    .n2     (sc_n2),
    .n4     (sc_n4),
    .n13    (sc_n13),
    .grid   (sc_grid),
    .idx    (cp_idx),
    .fails_o(copy_fails)
  );

  // Self-contained probes for the store-side and whole-read-side cousins
  // (Bug 3 / Bug 4 / Bug 5). Each drives its own container internally, so they
  // need no stimulus here and tb_top stays the sole elaboration root.
  packed_store_probe u_store (
    .fails_o(store_fails)
  );

  packed_whole_read_probe u_whole (
    .fails_o(whole_fails)
  );

  packed_grid2d_probe u_grid2d (
    .fails_o(grid2d_fails)
  );

  // Hoisted to module scope: SVTEST_INIT expands to a variable declaration,
  // which some tools flag when written inside an initial block.
  `SVTEST_INIT

  initial begin

    // -----------------------------------------------------------------------
    // Stimulus
    // -----------------------------------------------------------------------
    bus.slots[0].valid = 1'b1;
    bus.slots[0].dir   = 6'd39;
    bus.slots[0].mode  = 4'd7;
    bus.slots[1].dir   = 6'd56;
    bus.slots[1].mode  = 4'd11;
    bus.slots[2].dir   = 6'd46;
    bus.slots[2].mode  = 4'd5;
    bus.base           = 10'd475;

    // ---- scope-probe stimulus ---------------------------------------------
    sc_asc.slots[0] = '{valid: 1'b1, dir: 6'd39, mode: 4'd7};
    sc_asc.slots[1] = '{valid: 1'b0, dir: 6'd56, mode: 4'd11};
    sc_asc.slots[2] = '{valid: 1'b1, dir: 6'd46, mode: 4'd5};
    sc_asc.base     = 10'd475;

    sc_desc.slots[0] = '{valid: 1'b1, dir: 6'd39, mode: 4'd7};
    sc_desc.slots[1] = '{valid: 1'b0, dir: 6'd56, mode: 4'd11};
    sc_desc.slots[2] = '{valid: 1'b1, dir: 6'd46, mode: 4'd5};
    sc_desc.base     = 10'd475;

    sc_n2.slots[0].dir = 6'd11;  sc_n2.slots[1].dir = 6'd22;  sc_n2.base = 10'd0;

    sc_n4.slots[0].dir = 6'd11;  sc_n4.slots[1].dir = 6'd22;
    sc_n4.slots[2].dir = 6'd33;  sc_n4.slots[3].dir = 6'd44;  sc_n4.base = 10'd0;

    sc_n13.slots[1].dir = 6'd11; sc_n13.slots[2].dir = 6'd22;
    sc_n13.slots[3].dir = 6'd33; sc_n13.base = 10'd0;

    sc_grid.grid[0][0].dir = 6'd11; sc_grid.grid[0][1].dir = 6'd22;
    sc_grid.grid[1][0].dir = 6'd33; sc_grid.grid[1][1].dir = 6'd44;
    sc_grid.base = 10'd0;

    sc_arr[0].dir = 6'd39; sc_arr[1].dir = 6'd56; sc_arr[2].dir = 6'd46;

    sc_idx = 2'd1;
    cp_idx = 2'd0;   // idx==1 is the fixed point of the reversal, so use 0 here

    // ---- context A stimulus: same data on a top-module variable ------------
    ctx_top.slots[0] = '{valid: 1'b1, dir: 6'd39, mode: 4'd7};
    ctx_top.slots[1] = '{valid: 1'b0, dir: 6'd56, mode: 4'd11};
    ctx_top.slots[2] = '{valid: 1'b1, dir: 6'd46, mode: 4'd5};
    ctx_top.base     = 10'd475;
    ctx_idx          = 2'd0;

    tag = '{unit: 2'd2, seq: 7'd0, way: 3'd4}; // way[1:0] == 0 -> slot 0
    sel = 2'd0;
    for (int li = 0; li < 16; li++) begin
      obs[li] = sample_t'(100 + li);
    end

    #1; // let the continuous assigns settle

    // -----------------------------------------------------------------------
    // Control checks: shapes that must keep working everywhere
    // -----------------------------------------------------------------------
    `SVTEST_CHECK(base_out === 10'd475, "plain struct member read through continuous assign")
    `SVTEST_CHECK(bus.base === 10'd475, "plain struct member read in top module")

    // -----------------------------------------------------------------------
    // Bug 1: continuous-assign reads of packed-struct-array element members
    // -----------------------------------------------------------------------
    `SVTEST_CHECK(dir_member === 6'd39, "CA read bus.slots[tag.way[1:0]].dir with member part-select index")
    `SVTEST_CHECK(mode_member === 4'd7, "CA read bus.slots[tag.way[1:0]].mode with member part-select index")
    `SVTEST_CHECK(dir_const === 6'd39, "CA read bus.slots[0].dir with constant index")
    `SVTEST_CHECK(dir_var === 6'd39, "CA read bus.slots[sel].dir with variable index")

    // Move the selects around: way=5 -> way[1:0]==1 -> slot 1, sel -> slot 2.
    tag.way = 3'd5;
    sel     = 2'd2;
    #1;
    `SVTEST_CHECK(dir_member === 6'd56, "CA read bus.slots[tag.way[1:0]].dir after index change")
    `SVTEST_CHECK(dir_var === 6'd46, "CA read bus.slots[sel].dir after index change")

    // -----------------------------------------------------------------------
    // Bug 2: indexed reads of the packed-array field of an unpacked struct
    // (queue element -> pop -> module variable -> element read) inside an
    // instantiated monitor module
    // -----------------------------------------------------------------------
    @(posedge clk); #1 push_beat = 1;               // snapshot A queued at e2
    @(posedge clk); #1 pop_beat = 1;                // A queued at e2; at e3 the
                                                    // monitor pushes B and pops
                                                    // A on the same edge
    @(posedge clk); #1 push_beat = 0;               // B popped at e4
    @(posedge clk); #1 pop_beat = 0;
    #1;

    `SVTEST_CHECK(mon_fails == 0, "monitor lane compare of popped snapshot (indexed field reads)")

    // -----------------------------------------------------------------------
    // Bug 1 scope probe: which shapes of
    // `container.packed_array_of_struct[i].leaf` are mis-read
    // -----------------------------------------------------------------------
    `SVTEST_CHECK(scope_fails == 0, "scope probe: packed-array-of-struct member reads (compiled)")

    // -----------------------------------------------------------------------
    // Second-order finding: the same read gives a different answer depending on
    // the declaration context of the container. Three contexts, same type, same
    // data, all continuous assigns:
    //   A  top-module variable          -> shifted up by (N-1)*elem_w
    //   B  input port of a submodule    -> shifted up by (N-1)*elem_w  (u_scope)
    //   F  local aggregate in a submodule, continuously assigned from a port
    //                                   -> element order reversed  (u_copy)
    // -----------------------------------------------------------------------
    $display("TOP  asc  [0:2] .dir  : %0d %0d %0d  (exp 39 56 46)", top0, top1, top2);
    $display("TOP  asc  [0:2] .dir v: %0d  (exp 39, idx==0)", topv);

    `SVTEST_CHECK(top0 === 6'd39, "top: ctx_top.slots[0].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(top1 === 6'd56, "top: ctx_top.slots[1].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(top2 === 6'd46, "top: ctx_top.slots[2].dir (compiled, ascending [0:2])")
    `SVTEST_CHECK(topv === 6'd39, "top: ctx_top.slots[idx].dir with idx==0 (compiled)")

    `SVTEST_CHECK(copy_fails == 0, "copy probe: packed-array-of-struct member reads on a local copy (compiled)")

    // -----------------------------------------------------------------------
    // Cousins found while narrowing the read bug down (see README):
    //   Bug 3  procedural leaf stores through a packed-array member are dropped
    //          in an instantiated module (u_store)
    //   Bug 4  a whole-variable read of a container with an ascending
    //          array-of-struct member reverses the element order (u_whole)
    //   Bug 5  a 2-D packed array of packed structs member reads as 0 in an
    //          instantiated module (u_grid2d)
    // All three are correct when the very same module is the designated top.
    // -----------------------------------------------------------------------
    `SVTEST_CHECK(store_fails == 0, "store probe: leaf stores through a packed-array member")
    `SVTEST_CHECK(whole_fails == 0, "whole probe: whole-variable read of an ascending member")
    `SVTEST_CHECK(grid2d_fails == 0, "grid2d probe: 2-D packed array of structs member")

    // -----------------------------------------------------------------------
    // Report Test Success or Failure
    // -----------------------------------------------------------------------
    failures = failures + mon_fails + scope_fails + copy_fails
               + store_fails + whole_fails + grid2d_fails;
    `SVTEST_PASSFAIL
    // The shared SVTEST_PASSFAIL template only terminates on failure ($fatal).
    // This test has a free-running clock, so it must also stop explicitly on
    // the pass path, otherwise a working simulator runs forever.
    $finish;
  end

endmodule

