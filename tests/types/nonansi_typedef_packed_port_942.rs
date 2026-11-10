//! IEEE 1800-2023 §7.4.1 / §23.2.2.1 (#942): an ascending packed array of a
//! multi-bit typedef (`u8_t [0:7]`) declared as a NON-ANSI port keeps its
//! declared element order, as the same variable declaration does.
//!
//! The elaborator's non-ANSI port arms registered the packed dimensions only
//! for a plain `logic [..][..]` type. A typedef'd element carrying its own
//! dimensions (`output u8_t [0:7] o;`) recorded none, so the ascending range
//! fell back to `[7:0]` element order and every element came out mirrored
//! between the instance and the testbench (and through whole-vector copies).
//! An inlined instance also sized its non-ANSI ports before its own body
//! typedefs existed, so a module-local `typedef logic [7:0] u8_t` made each
//! element 32 bits (and broke even a descending `[7:0]` port), and a packed
//! array of a struct typedef recorded no shape at all.
//!
//! A module-local unpacked-array typedef (`typedef u8_t u8a_t [0:7]`) as a
//! function's return type, or a local declared with it, is resolved under
//! the instance's own key (section C5 of the report read x).
//!
//! The first design is the reporter's self-checking testbench (comments
//! stripped); the others are neighbouring shapes. Expected values are the
//! reference simulator's.

use xezim::simulate;

const REPORT: &str = r#"
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
int failures = 0;

`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end

`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

`endif

package MWE_TYPES;
   typedef logic [7:0] u8_t;
   typedef u8_t        u8a_t [0:7];
endpackage

module mwe_pref
  (
   tile_data,
   pad_active,
   main_wdata,
   above_wdata
   );
   import MWE_TYPES::*;

   input  wire [63:0] tile_data;
   input  wire        pad_active;
   output u8_t [0:7]  main_wdata;
   output u8_t [0:7]  above_wdata;

   always_comb begin
      main_wdata  = '0;
      above_wdata = '0;
      if (!pad_active) begin
         for (int wi = 0; wi < 8; wi++) begin
            main_wdata[wi]  = tile_data[wi*8 +: 8];
            above_wdata[wi] = tile_data[wi*8 +: 8];
         end
      end
   end
endmodule

module mwe_map
  (
   pq_live_wdata,
   live_wdata64,
   df_q_rdata
   );
   import MWE_TYPES::*;

   input  u8_t [0:7]   pq_live_wdata;
   output logic [63:0] live_wdata64;
   output u8_t [0:7]   df_q_rdata;

   assign live_wdata64 = pq_live_wdata;
   assign df_q_rdata   = live_wdata64;
endmodule

module vA_d;
   import MWE_TYPES::*;
   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t a [0:7];
   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         a[wi] = V[wi*8 +: 8];
   end
endmodule

module vB_d;
   import MWE_TYPES::*;
   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t a [7:0];
   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         a[wi] = V[wi*8 +: 8];
   end
endmodule

module vC_d;
   import MWE_TYPES::*;
   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t a [0:7];
   always_comb begin
      a[0] = V[63:56];
      a[1] = V[55:48];
      a[2] = V[47:40];
      a[3] = V[39:32];
      a[4] = V[31:24];
      a[5] = V[23:16];
      a[6] = V[15:8];
      a[7] = V[7:0];
   end
endmodule

module vD_d;
   import MWE_TYPES::*;
   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t a [0:7];
   initial begin
      for (int wi = 0; wi < 8; wi++)
         a[wi] = V[wi*8 +: 8];
   end
endmodule

module vE_d;
   import MWE_TYPES::*;
   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t b [0:7];
   u8_t a [0:7];
   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         b[wi] = V[wi*8 +: 8];
   end
   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         a[wi] = b[wi];
   end
endmodule

module vF_d;
   import MWE_TYPES::*;
   logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t  f1 [0:7];
   u8a_t f2;

   function automatic u8a_t mk(input logic [63:0] x);
      u8a_t r;
      for (int i = 0; i < 8; i++)
         r[i] = x[i*8 +: 8];
      return r;
   endfunction

   initial begin
      f1 = mk(V);
      f2 = mk(V);
   end
endmodule

module vH_d
  (
   tile_data,
   main_wdata
   );
   import MWE_TYPES::*;

   input  wire [63:0] tile_data;
   output u8_t [0:7]  main_wdata;

   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         main_wdata[wi] = tile_data[wi*8 +: 8];
   end
endmodule

module vQ_d
  (
   tile_data,
   main_wdata
   );
   typedef logic [7:0] u8_t;

   input  wire [63:0] tile_data;
   output u8_t [0:7]  main_wdata;

   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         main_wdata[wi] = tile_data[wi*8 +: 8];
   end
endmodule

module vT_d
  (
   main_wdata
   );
   typedef logic [7:0] u8_t;

   input u8_t [0:7] main_wdata;
endmodule

module vJ_d;
   typedef logic [7:0] u8_t;
   u8_t a [0:7];
   u8_t b [0:7];
   initial begin
      for (int wi = 0; wi < 8; wi++) a[wi] = 8'hC0 + wi[7:0];
      b = a;
   end
endmodule

module vO2_d
  (
   input  logic [63:0] tile_data,
   output MWE_TYPES::u8_t main_wdata [0:7]
   );
   always_comb begin
      for (int wi = 0; wi < 8; wi++)
         main_wdata[wi] = tile_data[wi*8 +: 8];
   end
endmodule

module w16_d
  (
   d
   );
   typedef logic [7:0] u8_t;
   output u8_t [0:15] d;
   always_comb begin
      for (int i = 0; i < 16; i++)
         d[i] = 8'hF0 + i[7:0];
   end
endmodule

module n2d_d
  (
   d
   );
   typedef logic [7:0] u8_t;
   output u8_t [0:1] [0:3] d;
   always_comb begin
      for (int i = 0; i < 2; i++)
         for (int j = 0; j < 4; j++)
            d[i][j] = 8'h10*i + 8'd0 + j[7:0];
   end
endmodule

module anp_d
  (
   output MWE_TYPES::u8_t [0:7] d
   );
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'hB0 + i[7:0];
   end
endmodule

module anu_d
  (
   output MWE_TYPES::u8a_t d
   );
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'hC0 + i[7:0];
   end
endmodule

module nad_d
  (
   d
   );
   typedef logic [7:0] u8_t;
   output u8_t [7:0] d;
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'hD0 + i[7:0];
   end
endmodule

module nad2_d
  (
   d
   );
   import MWE_TYPES::*;
   output u8_t [7:0] d;
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'h90 + i[7:0];
   end
endmodule

module vfl_d
  (
   );
   typedef logic [7:0] u8_t;
   typedef u8_t u8a_t [0:7];
   logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
   u8_t f [0:7];

   function automatic u8a_t mk(input logic [63:0] x);
      u8a_t r;
      for (int i = 0; i < 8; i++)
         r[i] = x[i*8 +: 8];
      return r;
   endfunction

   initial f = mk(V);
endmodule

module nau_d
  (
   d
   );
   typedef logic [7:0] u8_t;
   output u8_t d [0:7];
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'hE0 + i[7:0];
   end
endmodule

module nav_d
  (
   v
   );
   output wire [0:7] v;
   assign v = 8'hB4;
endmodule

module desc_d
  (
   d
   );
   typedef logic [7:0] u8_t;
   output u8_t [0:7] d;
   always_comb begin
      for (int i = 0; i < 8; i++)
         d[i] = 8'hA0 + i[7:0];
   end
endmodule

module tb_mwe_all;
   import MWE_TYPES::*;
   `SVTEST_INIT

   localparam logic [63:0] V = 64'hC0C1C2C3C8C9CACB;

   logic [63:0] tile_data;
   logic        pad_active;
   logic [63:0] o2_tile;

   u8_t [0:7]   mw_main, mw_above;
   logic [63:0] mw_live64;
   u8_t [0:7]   mw_q;

   u8_t [0:7]   vh_w, vh2_w, vq_w, vr_w, vr_snap, vt_w;

   u8_t         vo2_w [7:0];
   u8_t [0:15]  w16_w;
   u8_t [0:1] [0:3] n2d_w;
   u8_t [0:7]   anp_w, pos_w;
   u8a_t        anu_w, anu_snap;
   u8_t [7:0]   nad_w, desc_w, nad2_w;
   u8_t         nau_w [0:7];
   wire [0:7]   nav_v;

   mwe_pref u_pref (.tile_data(tile_data), .pad_active(pad_active),
                    .main_wdata(mw_main), .above_wdata(mw_above));
   mwe_map  u_map  (.pq_live_wdata(mw_main), .live_wdata64(mw_live64),
                    .df_q_rdata(mw_q));

   vH_d u_vH  (.tile_data(tile_data), .main_wdata(vh_w));
   vH_d u_vH2 (.tile_data(tile_data), .main_wdata(vh2_w));
   vQ_d u_vQ  (.tile_data(tile_data), .main_wdata(vq_w));
   vH_d u_vR  (.tile_data(tile_data), .main_wdata(vr_w));
   vT_d u_vT  (.main_wdata(vt_w));
   vO2_d u_vO2 (.tile_data(o2_tile), .main_wdata(vo2_w));

   w16_d u_w16 (.d(w16_w));
   n2d_d u_n2d (.d(n2d_w));
   anp_d u_anp (.d(anp_w));
   anu_d u_anu (.d(anu_w));
   nad_d u_nad (.d(nad_w));
   nad2_d u_nad2 (.d(nad2_w));
   nau_d u_nau (.d(nau_w));
   nav_d u_nav (.v(nav_v));
   desc_d u_desc (.d(desc_w));
   desc_d u_pos  (pos_w);

   vA_d u_vA ();
   vB_d u_vB ();
   vC_d u_vC ();
   vD_d u_vD ();
   vE_d u_vE ();
   vF_d u_vF ();
   vfl_d u_vfl ();
   vJ_d u_vJ ();

   function automatic u8_t [0:7] pack64(input logic [63:0] d);
      u8_t [0:7] v;
      int i;
      for (i = 0; i < 8; i++)
         v[i] = d[i*8 +: 8];
      return v;
   endfunction

   initial begin
      tile_data = 64'hCBCAC9C8C3C2C1C0;
      pad_active = 1'b0;
      o2_tile    = 64'hC7C6C5C4C3C2C1C0;
      for (int wi = 0; wi < 8; wi++)
         vt_w[wi] = 8'hC0 + wi[7:0];

      #1;

      $display("MWE: tile=%h main=%h live64=%h pack64=%h q=%h",
               tile_data, mw_main, mw_live64, pack64(tile_data), mw_q);
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(mw_main[wi] === tile_data[wi*8 +: 8],
                       "MWE: main_wdata element byte-reversed")
      `SVTEST_CHECK(mw_main === pack64(tile_data),
                    "MWE: always_comb fill != pack64() function fill")
      `SVTEST_CHECK(mw_above === pack64(tile_data),
                    "MWE: above_wdata word corrupted")
      `SVTEST_CHECK(mw_live64 === 64'hC0C1C2C3C8C9CACB,
                    "MWE: map unpacked->packed live_wdata64 byte-reversed")
      `SVTEST_CHECK(mw_q === pack64(tile_data),
                    "MWE: [pref_c0h0] Q readback byte-reversed (original fatal)")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vh_w[wi] === tile_data[wi*8 +: 8],
                       "vH: TB-side port-coupled element reversed")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vh2_w[wi] === tile_data[wi*8 +: 8],
                       "vH2: TB port-coupled read reversed")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vH2.main_wdata[wi] === tile_data[wi*8 +: 8],
                       "vH2: hierarchical read of DUT array wrong")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vq_w[wi] === tile_data[wi*8 +: 8],
                       "vQ: local-typedef non-ANSI port element reversed")

      vr_snap = vr_w;
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vr_w[wi] === tile_data[wi*8 +: 8],
                       "vR: element read of port-coupled array reversed")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vr_snap[wi] === tile_data[wi*8 +: 8],
                       "vR: whole-array snap read of port-coupled array reversed")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vT.main_wdata[wi] === 8'hC0 + wi[7:0],
                       "vT: DUT-side view of input port array reversed")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(vo2_w[wi] === o2_tile[(7-wi)*8 +: 8],
                       "vO2: mixed-bounds unpacked port mapping changed")

      for (int wi = 0; wi < 16; wi++)
         `SVTEST_CHECK(w16_w[wi] === 8'hF0 + wi[7:0],
                       "C1: 16-element ascending packed port mirrored")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(anp_w[wi] === 8'hB0 + wi[7:0],
                       "boundary: ANSI (header-declared) packed port must not be mirrored")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(pos_w[wi] === 8'hA0 + wi[7:0],
                       "C2: positional connection of ascending packed port mirrored")

      for (int i = 0; i < 2; i++)
         for (int j = 0; j < 4; j++)
            `SVTEST_CHECK(n2d_w[i][j] === 8'h10*i + 8'd0 + j[7:0],
                          "C3: 2-D packed non-ANSI port element garbage")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(desc_w[wi] === 8'hA0 + (7-wi),
                       "C4: mixed-direction packed coupling not bit-positional")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vfl.f[wi] === V[wi*8 +: 8],
                       "C5: function-returned unpacked array (local typedef) x")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(nad_w[wi] === 8'hD0 + wi[7:0],
                       "C6: non-unifying typedef sources break even [7:0] ports")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(nad2_w[wi] === 8'h90 + wi[7:0],
                       "boundary: descending packed port must not be mirrored")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(nau_w[wi] === 8'hE0 + wi[7:0],
                       "boundary: unpacked non-ANSI port must not be mirrored")

      anu_snap = anu_w;
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(anu_w[wi] === 8'hC0 + wi[7:0],
                       "boundary: ANSI unpacked typedef port must not be mirrored")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(anu_snap[wi] === 8'hC0 + wi[7:0],
                       "boundary: snap read of ANSI unpacked port array")

      `SVTEST_CHECK(nav_v[0] === 1'b1, "boundary: [0:7] vector MSB wrong")
      `SVTEST_CHECK(nav_v[7] === 1'b0, "boundary: [0:7] vector LSB wrong")
      `SVTEST_CHECK(nav_v === 8'hB4,   "boundary: [0:7] vector value wrong")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vA.a[wi] === V[wi*8 +: 8],
                       "vA: internal unpacked [0:7] always_comb fill wrong")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vB.a[wi] === V[wi*8 +: 8],
                       "vB: descending unpacked [7:0] fill wrong")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vC.a[wi] === V[63-8*wi -: 8],
                       "vC: constant-index fill wrong")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vD.a[wi] === V[wi*8 +: 8],
                       "vD: initial-block fill wrong")
      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vE.a[wi] === V[wi*8 +: 8],
                       "vE: plain-RHS element copy wrong")

      for (int wi = 0; wi < 8; wi++)
         `SVTEST_CHECK(u_vJ.b[wi] === u_vJ.a[wi],
                       "vJ: whole-array procedural copy mismatched")

      `SVTEST_PASSFAIL
      $finish;
   end
endmodule
"#;

/// Ascending `[1:8]`, continuous assigns, struct elements (package and
/// local typedef), a typedef'd array type, nested dims, input and inout
/// ports, a descending local-typedef port, a split declaration and a
/// function result.
const NEIGHBOURS: &str = r#"
package P;
  typedef logic [7:0] u8_t;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  typedef logic [3:0] n4_t;
endpackage
module o18(o); import P::*; output u8_t [1:8] o;
  always_comb for (int i = 1; i <= 8; i++) o[i] = 8'h10 + i[7:0];
endmodule
module oca(o); import P::*; output u8_t [0:3] o;
  assign o = 32'h01020304;
endmodule
module oea(o); import P::*; output u8_t [0:3] o;
  assign o[0] = 8'hA0; assign o[1] = 8'hA1; assign o[2] = 8'hA2; assign o[3] = 8'hA3;
endmodule
module ost(o); import P::*; output s_t [0:3] o;
  always_comb
    for (int i = 0; i < 4; i++) begin o[i].a = i[3:0]; o[i].b = 4'hF - i[3:0]; end
endmodule
module ostl(o);
  typedef struct packed { logic [3:0] a; logic [3:0] b; } ls_t;
  output ls_t [0:3] o;
  always_comb for (int i = 0; i < 4; i++) begin o[i].a = i[3:0]; o[i].b = 4'h5; end
endmodule
module otd(o); typedef logic [7:0] b_t; typedef b_t [0:3] w_t; output w_t o;
  always_comb for (int i = 0; i < 4; i++) o[i] = 8'hC0 + i[7:0];
endmodule
module onest(o); import P::*; output n4_t [0:1][0:2] o;
  always_comb
    for (int i = 0; i < 2; i++) for (int j = 0; j < 3; j++) o[i][j] = 4'(3*i + j);
endmodule
module iin(i, w, e0); import P::*;
  input u8_t [0:3] i; output logic [31:0] w; output u8_t e0;
  assign w = i; assign e0 = i[0];
endmodule
module ino(io); import P::*; inout wire u8_t [0:3] io;
  assign io = 32'hB0B1B2B3;
endmodule
module odl(o); typedef logic [7:0] u8_t; output u8_t [3:0] o;
  always_comb for (int i = 0; i < 4; i++) o[i] = 8'hD0 + i[7:0];
endmodule
module osplit(o); import P::*; output o; u8_t [0:3] o;
  always_comb for (int i = 0; i < 4; i++) o[i] = 8'hE0 + i[7:0];
endmodule
module ofn(o); import P::*; output u8_t [0:3] o;
  function automatic u8_t [0:3] mk();
    for (int k = 0; k < 4; k++) mk[k] = 8'h70 + k[7:0];
  endfunction
  assign o = mk();
endmodule
module tb; import P::*;
  u8_t [1:8] w18; u8_t [0:3] wca, wea, wtd, wsp, wfn; s_t [0:3] wst; logic [31:0] wstl;
  n4_t [0:1][0:2] wn; u8_t [0:3] iv; logic [31:0] iw; u8_t ie0; wire [31:0] wio;
  u8_t [3:0] wdl;
  o18 u1(.o(w18)); oca u2(.o(wca)); oea u3(.o(wea)); ost u4(.o(wst)); ostl u5(.o(wstl));
  otd u6(.o(wtd)); onest u7(.o(wn)); iin u8(.i(iv), .w(iw), .e0(ie0)); ino u9(.io(wio));
  odl u10(.o(wdl)); osplit u11(.o(wsp)); ofn u12(.o(wfn));
  initial begin
    iv = 32'h11223344;
    #1;
    $display("o18 %h %h %h %h", w18, w18[1], w18[8], u1.o[1]);
    $display("oca %h %h %h", wca, wca[0], u2.o[0]);
    $display("oea %h %h %h", wea, wea[0], u3.o[3]);
    $display("ost %h %h %h %h", wst, wst[0], wst[3].b, u4.o[1].a);
    $display("ostl %h %h", wstl, u5.o[0]);
    $display("otd %h %h %h", wtd, wtd[0], u6.o[0]);
    $display("onest %h %h %h %h", wn, wn[0][0], wn[1][2], u7.o[0][1]);
    $display("iin %h %h %h %h", iw, ie0, u8.i[0], u8.i[3]);
    $display("ino %h %h", wio, u9.io[0]);
    $display("odl %h %h %h", wdl, wdl[0], u10.o[0]);
    $display("osplit %h %h %h", wsp, wsp[0], u11.o[0]);
    $display("ofn %h %h %h", wfn, wfn[0], u12.o[0]);
  end
endmodule
"#;

/// Module-local unpacked-array typedefs as function results and locals.
const FUNCTION_RESULTS: &str = r#"
module d1;
  typedef logic [7:0] u8_t;
  typedef u8_t u8a_t [0:7];
  logic [63:0] V = 64'hC0C1C2C3C8C9CACB;
  u8_t f [0:7];
  u8a_t g;
  function automatic u8a_t mk(input logic [63:0] x);
    u8a_t r;
    for (int i = 0; i < 8; i++) r[i] = x[i*8 +: 8];
    return r;
  endfunction
  initial begin f = mk(V); g = mk(V); end
endmodule
module d3;
  typedef logic [7:0] v8a_t [0:3];
  logic [7:0] f [0:3];
  logic [7:0] g [0:3];
  v8a_t h;
  function automatic v8a_t mk(); for (int i = 0; i < 4; i++) mk[i] = 8'hC0 + i; endfunction
  function automatic v8a_t mk2(); v8a_t r; r[1] = 8'h55; return r; endfunction
  function automatic v8a_t mk3(); return '{8'h1, 8'h2, 8'h3, 8'h4}; endfunction
  initial begin f = mk(); g = mk2(); h = mk3(); end
endmodule
module tb;
  d1 u1();
  d3 u3();
  initial begin
    #1;
    $display("d1 %h %h %h %h", u1.f[0], u1.f[7], u1.g[0], u1.g[7]);
    $display("d3 %h %h | %h %h | %h %h", u3.f[0], u3.f[3], u3.g[0], u3.g[1], u3.h[0],
             u3.h[3]);
  end
endmodule
"#;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(out: &[String], want: &[&str]) {
    for w in want {
        assert!(out.iter().any(|l| l == w), "missing {w:?} in {out:?}");
    }
}

#[test]
fn reporter_testbench_passes() {
    let out = lines(REPORT);
    let fails: Vec<&String> = out.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "failures: {fails:?}");
    expect(&out, &["TEST_PASS"]);
    expect(
        &out,
        &[
            "MWE: tile=cbcac9c8c3c2c1c0 main=c0c1c2c3c8c9cacb live64=c0c1c2c3c8c9cacb \
           pack64=c0c1c2c3c8c9cacb q=c0c1c2c3c8c9cacb",
        ],
    );
}

#[test]
fn neighbouring_port_shapes() {
    expect(
        &lines(NEIGHBOURS),
        &[
            "o18 1112131415161718 11 18 11",
            "oca 01020304 01 01",
            "oea a0a1a2a3 a0 a3",
            "ost 0f1e2d3c 0f c 1",
            "ostl 05152535 05",
            "otd c0c1c2c3 c0 c0",
            "onest 012345 0 5 1",
            "iin 11223344 11 11 44",
            "ino b0b1b2b3 b0",
            "odl d3d2d1d0 d0 d0",
            "osplit e0e1e2e3 e0 e0",
            "ofn 70717273 70 70",
        ],
    );
}

#[test]
fn local_unpacked_typedef_function_results() {
    expect(
        &lines(FUNCTION_RESULTS),
        &["d1 cb c0 cb c0", "d3 c0 c3 | xx 55 | 01 04"],
    );
}
