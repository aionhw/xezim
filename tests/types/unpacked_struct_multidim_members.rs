//! Members of an UNPACKED struct with two or more unpacked dimensions
//! (`bit [7:0] mm [2][2]`, `m3 [2][2][2]`).
//!
//! Every element of such a member is its own leaf, keyed by the full index
//! list. The shared helper that enumerates a member's elements reported only
//! the FIRST unpacked dimension, so every consumer that walked it — the
//! whole-struct copy (module, block-local, nested, array element, queue
//! element), block-local and formal leaf seeding, assignment patterns and
//! `%p` — saw `mm[0]`, `mm[1]` and lost the member. Element selects on a
//! member reached through an index or a synthesized member access, and a
//! nonblocking whole-struct assignment, lost it as well.
//!
//! Expected values are the reference simulator's.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn tagged(src: &str) -> Vec<String> {
    out(src)
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// The user's testcase: copies, locals, formals, patterns, queues and `%p`.
#[test]
fn multidim_member_survives_every_operation() {
    let o: Vec<String> = out(USER_TB)
        .into_iter()
        .filter(|l| l.contains("obs") || l.contains("TEST_") || l.contains("FAIL"))
        .collect();
    assert_eq!(
        o,
        [
            "  obs Lg   m.mm[1][1] = d3 (want d3)",
            "  obs B1  ub.mm[1][1] = 33 (want 33)",
            "  obs B2  ub.mm[1][1] = 33 (want 33)",
            "  obs C1  mC1.mm[1][1] = ee (want ee)",
            "  obs C2  pC2.pm[1]   = d9 (want d9)",
            "  obs C2b pb.pm[0]    = 77 (want 77)",
            "  obs C3  loc.mm[1][1] = 5a (want 5a)",
            "  obs K1s  a.sd[1]    = 23 (want 23)",
            "  obs K1m  a.mm[1][1] = 33 (want 33)",
            "  obs K2s  ur.sd[1]    = b8 (want b8)",
            "  obs Cb0  uCb.sd[1]   = 23 (want 23)",
            "  obs Cb1  uCb.mm[0][0]/[1][1] = 24 27 (want 24 27)",
            "  obs Cb2s uCb2.sd[1]   = 33 (want 33)",
            "  obs Cb2  uCb2.mm[0][0]/[1][1] = 34 37 (want 34 37)",
            "  obs Ce1  ma.body.mm[1][1] = 44 (want 44)",
            "  obs Ce2  mb.body.mm[1][1] = 44 (want 44)",
            "  obs Ce3  uCe3.mm[1][1] = 44 (want 44)",
            "  obs Ce4  mb2.body.mm[1][1] = 33 (want 33)",
            "  obs Ch0  ub.sc / ub.sd[1] = a1 23 (want a1 23)",
            "  obs Ch   ub.mm[1][1] = 33 (want 33)",
            "  obs Ci2  ub3.m3[1][1][1] = 66 (want 66)",
            "  obs Cc0  uarr[1].sd[1]/mm[1][1] = 53 43 (want 53 43)",
            "  obs Cc3  uarr[0].mm[1][1] = 33 (want 33)",
            "  obs Cc1  a[1].mm[1][1] = 43 (want 43)",
            "  obs Cd1  uCd1.mm[1][1] = 33 (want 33)",
            "  obs Cd2  uq[0].mm[1][1] = 33 (want 33)",
            "  obs Cd3  uq[0].mm[1][1] = 88 (want 88)",
            "  obs Cd3b uq[1].mm[1][1] = 33 (want 33)",
            "  obs Cf   '{sc:'ha1, sd:'{'h22, 'h23}, mm:'{'{'h30, 'h31}, '{'h32, 'h33}}}",
            "  obs Cg   '{'{'h30, 'h31}, '{'h32, 'h33}}",
            "  obs CfS  ['{sc:'ha1, sd:'{'h22, 'h23}, mm:'{'{'h30, 'h31}, '{'h32, 'h33}}}]",
            "  obs Lc   mLc.mm[i][j] = e1 (want e1)",
            "  obs Ld   pLd.pm[1][7:4] = f (want f)",
            "  obs Le   sum = 198 (want 198)",
            "  obs Lf   arrLf[1].mm[1][1] = e5 (want e5)",
            "  obs L0   pUniq.pm[0] = 77 (want 77)",
            "  obs L2b  p.pm[1] = 00 (want anything but 55)",
            "  obs L2d  p.pm[0] = 00 (want anything but 99)",
            "TEST_PASS",
            "  obs Lh   m.mm[1][1] = d4 (want d4) [final scope]",
        ],
        "{o:#?}"
    );
}

const USER_TB: &str = r#"
`timescale 1ns/1ps

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT int failures = 0;

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

// ----- member families under test -----
typedef struct {
  bit [7:0] sc;           // scalar member
  bit [7:0] sd [2];       // ONE unpacked dimension
  bit [7:0] mm [2][2];    // TWO unpacked dimensions
} US;

typedef struct {
  bit [7:0]      sc;
  bit [1:0][7:0] pm;      // packed 2-D member
} PS;

typedef struct {
  bit [7:0] hdr;
  US      body;           // nested struct containing a multi-dim member
} MS;

typedef struct {
  bit [7:0] m3 [2][2][2]; // THREE unpacked dimensions
} US3;

class CFill;
  US s;
  function void fill();
    s.sc       = 8'hA1;
    s.mm[0][0] = 8'h50;
    s.mm[0][1] = 8'h51;
    s.mm[1][0] = 8'h59;
    s.mm[1][1] = 8'h5A;
  endfunction
endclass

module tb;
  `SVTEST_INIT

  // ---------- variables (module scope) ----------
  US  ua, ub, uw, uc, ur;
  PS  pa, pb;
  MS  ma, mb, mb2;
  US3 u3, ub3;
  US  uarr[2];
  US  uq[$];
  CFill c;

  // ---------- SECTION B support: scalar-struct formal ----------
  task automatic tform(US a);
    $display("  obs K1s  a.sd[1]    = %02h (want 23)", a.sd[1]);
    `SVTEST_CHECK(a.sd[1] === 8'h23, "K1s: struct formal keeps single-dim member")
    $display("  obs K1m  a.mm[1][1] = %02h (want 33)", a.mm[1][1]);
    `SVTEST_CHECK(a.mm[1][1] === 8'h33, "K1m: struct formal keeps multi-dim member")
  endtask

  // ---------- SECTION B support: function return ----------
  function automatic US fret();
    US r;
    r.sc    = 8'hA7;
    r.sd[1] = 8'hB8;
    return r;
  endfunction

  // ---------- SECTION B support: array-of-struct formal ----------
  task automatic tarr(US a[2]);
    $display("  obs Cc1  a[1].mm[1][1] = %02h (want 43)", a[1].mm[1][1]);
    `SVTEST_CHECK(a[1].mm[1][1] === 8'h43, "Cc1: array-of-struct formal keeps multi-dim member")
    `SVTEST_CHECK(a[1].sd[1] === 8'h53, "Cc2: array-of-struct formal keeps single-dim member")
    `SVTEST_CHECK(a[0].mm[1][1] === 8'h33, "Cc4: array-of-struct formal sees copied element")
  endtask

  // ---------- SECTION B: always-block local (executes at t0; sim ends
  //            at $finish) ----------
  always begin : aLg
    US m;
    m.mm[1][1] = 8'hD3;
    $display("  obs Lg   m.mm[1][1] = %02h (want d3)", m.mm[1][1]);
    `SVTEST_CHECK(m.mm[1][1] === 8'hD3, "Lg: always-block local multi-dim element r/w")
    #1000;
  end

  // ---------- SECTION B: final-block local (runs at $finish, AFTER the
  //            summary) ----------
  final begin : aLh
    US m;
    m.mm[1][1] = 8'hD4;
    $display("  obs Lh   m.mm[1][1] = %02h (want d4) [final scope]", m.mm[1][1]);
    if (!(m.mm[1][1] === 8'hD4)) begin
      failures++;
      $display("FAIL @final : Lh: final-block local multi-dim element r/w");
    end
  end

  initial begin : main
    c = new();
    c.fill();

    // A1: module-scope multi-dim member element r/w (control)
    ua.mm[1][1] = 8'h33;
    `SVTEST_CHECK(ua.mm[1][1] === 8'h33, "A1: module var multi-dim element r/w")

    // A2: module-scope packed-2D member element r/w (control)
    pa.pm[1] = 8'h55;
    `SVTEST_CHECK(pa.pm[1] === 8'h55, "A2: module var packed-2D element r/w")

    // B1: module -> module whole-struct copy must keep multi-dim member
    ub = ua;
    $display("  obs B1  ub.mm[1][1] = %02h (want 33)", ub.mm[1][1]);
    `SVTEST_CHECK(ub.mm[1][1] === 8'h33, "B1: module->module struct copy keeps multi-dim member")

    // B2: block-local copy-in -> copy-out round trip, multi-dim member
    begin : blkB2
      US u;
      u = ua;
      ub = u;
    end
    $display("  obs B2  ub.mm[1][1] = %02h (want 33)", ub.mm[1][1]);
    `SVTEST_CHECK(ub.mm[1][1] === 8'h33, "B2: local round trip keeps multi-dim member")

    // B3: same round trip, packed-2D member (control)
    begin : blkB3
      PS pB3;
      pB3 = pa;
      pb = pB3;
    end
    `SVTEST_CHECK(pb.pm[1] === 8'h55, "B3: local round trip keeps packed-2D member")

    // C1: block-local multi-dim element write -> read
    begin : blkC1
      US mC1;
      mC1.mm[1][1] = 8'hEE;
      $display("  obs C1  mC1.mm[1][1] = %02h (want ee)", mC1.mm[1][1]);
      `SVTEST_CHECK(mC1.mm[1][1] === 8'hEE, "C1: block-local multi-dim element write->read")
    end

    // C2: block-local packed-2D element write -> read
    begin : blkC2
      PS pC2;
      pC2.pm[1] = 8'hD9;
      $display("  obs C2  pC2.pm[1]   = %02h (want d9)", pC2.pm[1]);
      `SVTEST_CHECK(pC2.pm[1] === 8'hD9, "C2: block-local packed-2D element write->read")
    end

    // C2b: block-local packed-2D write, copy out, read at module scope.
    begin : blkC2b
      PS pC2b;
      pC2b.pm[0] = 8'h77;
      pb = pC2b;
    end
    $display("  obs C2b pb.pm[0]    = %02h (want 77)", pb.pm[0]);
    `SVTEST_CHECK(pb.pm[0] === 8'h77, "C2b: block-local packed-2D write lands in storage")

    // C2c: block-local packed-2D read after copy-in (control)
    begin : blkC2c
      PS pC2c;
      pC2c = pa;
      `SVTEST_CHECK(pC2c.pm[1] === 8'h55, "C2c: block-local packed-2D read after copy-in")
    end

    // C3: block-local multi-dim read after a class-property copy-out.
    begin : blkC3
      US loc;
      loc = c.s;
      $display("  obs C3  loc.mm[1][1] = %02h (want 5a)", loc.mm[1][1]);
      `SVTEST_CHECK(loc.mm[1][1] === 8'h5A, "C3: block-local multi-dim read after class copy-out")
    end

    // C3sib: same copy-out to a module-scope variable (control)
    uw = c.s;
    `SVTEST_CHECK(uw.mm[1][1] === 8'h5A, "C3sib: module-scope multi-dim read after class copy-out")

    // D1: block-local single-dim member element r/w (control)
    begin : blkD1
      US mD1;
      mD1.sd[1] = 8'h5C;
      `SVTEST_CHECK(mD1.sd[1] === 8'h5C, "D1: block-local single-dim element r/w")
    end

    // D2: block-local scalar member r/w (control)
    begin : blkD2
      US mD2;
      mD2.sc = 8'h7E;
      `SVTEST_CHECK(mD2.sc === 8'h7E, "D2: block-local scalar member r/w")
    end

    // ---------- staging: module-scope element writes ----------
    ua.sc = 8'hA1;  ua.sd[0] = 8'h22;  ua.sd[1] = 8'h23;
    ua.mm[0][0] = 8'h30;  ua.mm[0][1] = 8'h31;
    ua.mm[1][0] = 8'h32;  ua.mm[1][1] = 8'h33;
    pa.pm[0] = 8'h11;  pa.pm[1] = 8'h55;
    uc.mm[1][1] = 8'h88;

    `SVTEST_CHECK(ua.mm[1][1] === 8'h33, "A1: module var multi-dim element r/w (control)")
    `SVTEST_CHECK(pa.pm[1]   === 8'h55, "A2: module var packed-2D element r/w (control)")

    // ================= S0: formals & function returns =================
    tform(ua);

    ur = fret();
    $display("  obs K2s  ur.sd[1]    = %02h (want b8)", ur.sd[1]);
    `SVTEST_CHECK(ur.sd[1] === 8'hB8, "K2s: struct return keeps single-dim member")

    // ================= S1: copy machinery =================
    begin : blkCb1
      US uCb = '{sc: 8'h21, sd: '{8'h22, 8'h23},
                 mm: '{'{8'h24, 8'h25}, '{8'h26, 8'h27}}};
      $display("  obs Cb0  uCb.sd[1]   = %02h (want 23)", uCb.sd[1]);
      `SVTEST_CHECK(uCb.sd[1] === 8'h23, "Cb0: pattern decl-init fills single-dim member")
      $display("  obs Cb1  uCb.mm[0][0]/[1][1] = %02h %02h (want 24 27)", uCb.mm[0][0], uCb.mm[1][1]);
      `SVTEST_CHECK(uCb.mm[1][1] === 8'h27, "Cb1: pattern decl-init fills multi-dim member")
    end

    begin : blkCb2
      US uCb2;
      uCb2 = '{sc: 8'h31, sd: '{8'h32, 8'h33},
               mm: '{'{8'h34, 8'h35}, '{8'h36, 8'h37}}};
      $display("  obs Cb2s uCb2.sd[1]   = %02h (want 33)", uCb2.sd[1]);
      `SVTEST_CHECK(uCb2.sd[1] === 8'h33, "Cb2s: pattern assignment fills single-dim member")
      $display("  obs Cb2  uCb2.mm[0][0]/[1][1] = %02h %02h (want 34 37)", uCb2.mm[0][0], uCb2.mm[1][1]);
      `SVTEST_CHECK(uCb2.mm[1][1] === 8'h37, "Cb2: pattern assignment fills multi-dim member")
    end

    ma.body.mm[1][1] = 8'h44;
    $display("  obs Ce1  ma.body.mm[1][1] = %02h (want 44)", ma.body.mm[1][1]);
    `SVTEST_CHECK(ma.body.mm[1][1] === 8'h44, "Ce1: module var nested multi-dim element r/w")

    mb = ma;
    $display("  obs Ce2  mb.body.mm[1][1] = %02h (want 44)", mb.body.mm[1][1]);
    `SVTEST_CHECK(mb.body.mm[1][1] === 8'h44, "Ce2: nested-struct copy keeps inner multi-dim member")

    begin : blkCe3
      US uCe3;
      uCe3 = mb.body;
      $display("  obs Ce3  uCe3.mm[1][1] = %02h (want 44)", uCe3.mm[1][1]);
      `SVTEST_CHECK(uCe3.mm[1][1] === 8'h44, "Ce3: member copy-out keeps multi-dim member")
    end

    mb2.body = ua;
    $display("  obs Ce4  mb2.body.mm[1][1] = %02h (want 33)", mb2.body.mm[1][1]);
    `SVTEST_CHECK(mb2.body.mm[1][1] === 8'h33, "Ce4: member copy-in keeps multi-dim member")

    ub <= ua;
    #1;
    $display("  obs Ch0  ub.sc / ub.sd[1] = %02h %02h (want a1 23)", ub.sc, ub.sd[1]);
    `SVTEST_CHECK(ub.sc    === 8'hA1, "Ch00: NBA copy keeps scalar member")
    `SVTEST_CHECK(ub.sd[1] === 8'h23, "Ch0: NBA copy keeps single-dim member")
    $display("  obs Ch   ub.mm[1][1] = %02h (want 33)", ub.mm[1][1]);
    `SVTEST_CHECK(ub.mm[1][1] === 8'h33, "Ch: NBA copy keeps multi-dim member")

    u3.m3[1][1][1] = 8'h66;
    `SVTEST_CHECK(u3.m3[1][1][1] === 8'h66, "Ci1: module var 3-dim element r/w")
    ub3 = u3;
    $display("  obs Ci2  ub3.m3[1][1][1] = %02h (want 66)", ub3.m3[1][1][1]);
    `SVTEST_CHECK(ub3.m3[1][1][1] === 8'h66, "Ci2: struct copy keeps 3-dim member")

    uarr[1].sd[1]    = 8'h53;
    uarr[1].mm[1][1] = 8'h43;
    $display("  obs Cc0  uarr[1].sd[1]/mm[1][1] = %02h %02h (want 53 43)", uarr[1].sd[1], uarr[1].mm[1][1]);
    `SVTEST_CHECK(uarr[1].mm[1][1] === 8'h43, "Cc0: module var array-of-struct element multi-dim r/w")
    uarr[0] = ua;
    $display("  obs Cc3  uarr[0].mm[1][1] = %02h (want 33)", uarr[0].mm[1][1]);
    `SVTEST_CHECK(uarr[0].mm[1][1] === 8'h33, "Cc3: copy into array element keeps multi-dim member")
    tarr(uarr);

    uq.push_back(ua);
    begin : blkCd1
      US uCd1;
      uCd1 = uq.pop_front();
      $display("  obs Cd1  uCd1.mm[1][1] = %02h (want 33)", uCd1.mm[1][1]);
      `SVTEST_CHECK(uCd1.mm[1][1] === 8'h33, "Cd1: queue push/pop keeps multi-dim member")
    end

    uq.push_back(ua);
    $display("  obs Cd2  uq[0].mm[1][1] = %02h (want 33)", uq[0].mm[1][1]);
    `SVTEST_CHECK(uq[0].mm[1][1] === 8'h33, "Cd2: queue element read keeps multi-dim member")

    uq.delete();
    uq.push_back(ua);
    uq.push_back(uc);
    uq.reverse();
    `SVTEST_CHECK(uq.size() === 2, "Cd3s: queue reverse keeps size")
    $display("  obs Cd3  uq[0].mm[1][1] = %02h (want 88)", uq[0].mm[1][1]);
    `SVTEST_CHECK(uq[0].mm[1][1] === 8'h88, "Cd3a: queue reverse reorders (first)")
    $display("  obs Cd3b uq[1].mm[1][1] = %02h (want 33)", uq[1].mm[1][1]);
    `SVTEST_CHECK(uq[1].mm[1][1] === 8'h33, "Cd3b: queue reverse reorders (second)")

    $display("  obs Cf   %p", ua);
    $display("  obs Cg   %p", ua.mm);
    begin : blkCfs
      string sf;
      sf = $sformatf("%p", ua);
      $display("  obs CfS  [%s]", sf);
    end

    begin : blkLc
      US  mLc;
      int i, j;
      i = 1;
      j = 1;
      mLc.mm[i][j] = 8'hE1;
      $display("  obs Lc   mLc.mm[i][j] = %02h (want e1)", mLc.mm[i][j]);
      `SVTEST_CHECK(mLc.mm[i][j] === 8'hE1, "Lc: block-local multi-dim r/w with variable indices")
    end

    begin : blkLd
      PS pLd;
      pLd = pa;
      pLd.pm[1][7:4] = 4'hF;
      $display("  obs Ld   pLd.pm[1][7:4] = %01h (want f)", pLd.pm[1][7:4]);
      `SVTEST_CHECK(pLd.pm[1][7:4] === 4'hF, "Ld: block-local packed-2D part-select write->read")
      `SVTEST_CHECK(pLd.pm[1][3:0] === 4'h5, "Ld2: part-select write leaves low bits intact")
    end

    begin : blkLe
      int sum;
      sum = 0;
      foreach (ua.mm[i, j]) sum = sum + ua.mm[i][j];
      $display("  obs Le   sum = %0d (want 198)", sum);
      `SVTEST_CHECK(sum == 198, "Le: foreach over multi-dim member reads all elements")
    end

    begin : blkLf
      US arrLf[2];
      arrLf[1].mm[1][1] = 8'hE5;
      $display("  obs Lf   arrLf[1].mm[1][1] = %02h (want e5)", arrLf[1].mm[1][1]);
      `SVTEST_CHECK(arrLf[1].mm[1][1] === 8'hE5, "Lf: block-local array-of-struct element multi-dim r/w")
    end

    begin : blkL0
      PS pUniq;
      pUniq.pm[0] = 8'h77;
      $display("  obs L0   pUniq.pm[0] = %02h (want 77)", pUniq.pm[0]);
      `SVTEST_CHECK(pUniq.pm[0] === 8'h77, "L0: unique-named local packed-2D write")
    end

    begin : lk1
      PS p;
      p = pa;
    end
    begin : lk2
      PS p;
      p.pm[0] = 8'h99;
      `SVTEST_CHECK(p.pm[0] === 8'h99, "L2a: same-named local write works (leak-masked)")
      $display("  obs L2b  p.pm[1] = %02h (want anything but 55)", p.pm[1]);
      `SVTEST_CHECK(!(p.pm[1] === 8'h55), "L2b: fresh same-named local not primed by earlier sibling")
      `SVTEST_CHECK(pa.pm[1] === 8'h55, "L2c: module var unaffected by same-named local writes")
      `SVTEST_CHECK(pa.pm[0] === 8'h11, "L2c2: module var low element unaffected")
    end
    begin : lk3
      PS p;
      $display("  obs L2d  p.pm[0] = %02h (want anything but 99)", p.pm[0]);
      `SVTEST_CHECK(!(p.pm[0] === 8'h99), "L2d: third same-named local does not inherit prior write")
    end

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
"#;

/// Mixed widths, descending and 3-D dimensions: copy, `%p` (left bound
/// first), `==`, assignment patterns, and a class property copied in and out.
#[test]
fn mixed_width_and_descending_members() {
    let o = tagged(
        r#"
// Mixed widths, descending and 3-D member dimensions: copy, %p, ==,
// assignment patterns (left bound first), and a class property copied in
// and out.
typedef struct {
  logic [3:0]  a [1:0][0:2];
  bit   [15:0] b [3][2];
  int          c;
  byte         d [2][1:0][2];
} mx_t;

typedef struct {
  bit [7:0] sd [1:0];
  bit [7:0] mm [1:0][0:1];
} ds_t;

class holder_c;
  mx_t m;
endclass

module tb;
  mx_t xa, xb;
  ds_t da, db;
  holder_c h;
  initial begin
    foreach (xa.a[i, j]) xa.a[i][j] = 4'(i * 3 + j);
    foreach (xa.b[i, j]) xa.b[i][j] = 16'h100 * i + j;
    xa.c = -5;
    foreach (xa.d[i, j, k]) xa.d[i][j][k] = 8'(i * 16 + j * 4 + k);
    xb = xa;
    $display("T|copy a10=%h a02=%h b21=%h c=%0d d111=%h d010=%h",
             xb.a[1][0], xb.a[0][2], xb.b[2][1], xb.c, xb.d[1][1][1], xb.d[0][1][0]);
    $display("T|p %p", xb);
    $display("T|pd %p", xa.d);
    $display("T|eq %0d %0d", xa == xb, xa != xb);
    xb.a[1][2] = 4'hF;
    $display("T|eq2 %0d %0d", xa == xb, xa != xb);
    h = new();
    h.m = xa;
    xb = h.m;
    $display("T|cls a11=%h b20=%h d101=%h", xb.a[1][1], xb.b[2][0], xb.d[1][0][1]);
    da = '{sd: '{8'h11, 8'h22}, mm: '{'{8'h31, 8'h32}, '{8'h33, 8'h34}}};
    $display("T|pat sd0=%h sd1=%h mm00=%h mm01=%h mm10=%h mm11=%h",
             da.sd[0], da.sd[1], da.mm[0][0], da.mm[0][1], da.mm[1][0], da.mm[1][1]);
    db = da;
    $display("T|dp %p", db);
    db.mm = '{'{8'h9, 8'h8}, '{8'h7, 8'h6}};
    db.mm[0] = '{8'hC, 8'hD};
    $display("T|mmpat %p", db.mm);
    db = '{default: 8'h5};
    $display("T|dflt %p", db);
  end
endmodule
"#,
    );
    assert_eq!(
        o,
        [
            "T|copy a10=3 a02=2 b21=0201 c=-5 d111=15 d010=04",
            "T|p '{a:'{'{'h3, 'h4, 'h5}, '{'h0, 'h1, 'h2}}, b:'{'{'h0, 'h1}, '{'h100, 'h101}, '{'h200, 'h201}}, c:-5, d:'{'{'{4, 5}, '{0, 1}}, '{'{20, 21}, '{16, 17}}}}",
            "T|pd '{'{'{4, 5}, '{0, 1}}, '{'{20, 21}, '{16, 17}}}",
            "T|eq 1 0",
            "T|eq2 0 1",
            "T|cls a11=4 b20=0200 d101=11",
            "T|pat sd0=22 sd1=11 mm00=33 mm01=34 mm10=31 mm11=32",
            "T|dp '{sd:'{'h11, 'h22}, mm:'{'{'h31, 'h32}, '{'h33, 'h34}}}",
            "T|mmpat '{'{'h9, 'h8}, '{'hc, 'hd}}",
            "T|dflt '{sd:'{'h5, 'h5}, mm:'{'{'h5, 'h5}, '{'h5, 'h5}}}",
        ],
        "{o:#?}"
    );
}

/// Input, method, output, inout and ref formals, a function return filled by
/// `foreach`, a block local with variable indices, and the array queries.
#[test]
fn multidim_member_through_formals_and_queries() {
    let o = tagged(
        r#"
// A multi-dimensional member through formals, returns, locals and the
// array query functions.
typedef struct { bit [7:0] sc; bit [7:0] sd [2]; bit [7:0] mm [2][2]; } us_t;

class proc_c;
  bit [7:0] got;
  function void take(us_t a);
    got = a.mm[1][1];
    $display("T|meth mm11=%h mm01=%h sd1=%h sc=%h", a.mm[1][1], a.mm[0][1], a.sd[1], a.sc);
  endfunction
  task automatic tout(output us_t o);
    o.mm[0][1] = 8'hB1;
    o.sd[0] = 8'hB0;
  endtask
endclass

module tb;
  us_t ua, ub, uc;
  proc_c pr;

  function automatic us_t mk(input bit [7:0] base);
    us_t r;
    r.sc = base;
    foreach (r.mm[i, j]) r.mm[i][j] = base + 8'(i * 2 + j);
    return r;
  endfunction

  task automatic tin(us_t a);
    $display("T|in mm11=%h mm01=%h sd1=%h", a.mm[1][1], a.mm[0][1], a.sd[1]);
    $display("T|inp %p", a);
  endtask

  task automatic tio(inout us_t r);
    r.sd[1] = 8'hBD;
    r.mm[1][0] = 8'hBE;
  endtask

  task automatic tref(ref us_t r);
    r.sd[0] = 8'hAD;
    r.mm[0][0] = 8'hAE;
    r.sc = 8'hAC;
  endtask

  initial begin
    ua.sc = 8'hA1; ua.sd[0] = 8'h22; ua.sd[1] = 8'h23;
    ua.mm[0][0] = 8'h30; ua.mm[0][1] = 8'h31; ua.mm[1][0] = 8'h32; ua.mm[1][1] = 8'h33;
    tin(ua);
    pr = new();
    pr.take(ua);
    $display("T|got %h", pr.got);
    pr.tout(uc);
    $display("T|mout mm01=%h sd0=%h", uc.mm[0][1], uc.sd[0]);
    ub = mk(8'h60);
    $display("T|ret %p", ub);
    tio(ub);
    $display("T|inout %p", ub);
    tref(ub);
    $display("T|ref %p", ub);
    $display("T|q bits=%0d size=%0d size2=%0d dims=%0d udims=%0d bitsS=%0d",
             $bits(ua.mm), $size(ua.mm), $size(ua.mm, 2), $dimensions(ua.mm),
             $unpacked_dimensions(ua.mm), $bits(ua));
    $display("T|q1 size=%0d bits=%0d left=%0d right=%0d", $size(ua.mm[1]), $bits(ua.mm[1]),
             $left(ua.mm[1]), $right(ua.mm[1]));
    begin : blk
      us_t l;
      int i, j;
      i = 1; j = 0;
      l.mm[i][j] = 8'hE1;
      l.mm[0][1] = 8'hE2;
      $display("T|local %h %h size=%0d size2=%0d %p", l.mm[i][j], l.mm[0][1],
               $size(l.mm), $size(l.mm, 2), l);
    end
  end
endmodule
"#,
    );
    assert_eq!(
        o,
        [
            "T|in mm11=33 mm01=31 sd1=23",
            "T|inp '{sc:'ha1, sd:'{'h22, 'h23}, mm:'{'{'h30, 'h31}, '{'h32, 'h33}}}",
            "T|meth mm11=33 mm01=31 sd1=23 sc=a1",
            "T|got 33",
            "T|mout mm01=b1 sd0=b0",
            "T|ret '{sc:'h60, sd:'{'h0, 'h0}, mm:'{'{'h60, 'h61}, '{'h62, 'h63}}}",
            "T|inout '{sc:'h60, sd:'{'h0, 'hbd}, mm:'{'{'h60, 'h61}, '{'hbe, 'h63}}}",
            "T|ref '{sc:'hac, sd:'{'had, 'hbd}, mm:'{'{'hae, 'h61}, '{'hbe, 'h63}}}",
            "T|q bits=32 size=2 size2=2 dims=3 udims=2 bitsS=56",
            "T|q1 size=2 bits=16 left=0 right=1",
            "T|local e1 e2 size=2 size2=2 '{sc:'h0, sd:'{'h0, 'h0}, mm:'{'{'h0, 'he2}, '{'he1, 'h0}}}",
        ],
        "{o:#?}"
    );
}

/// Associative, dynamic and queue containers of the struct, and a local
/// array of structs. `num()`/`foreach` count keys, not member leaves.
#[test]
fn multidim_member_in_containers() {
    let o = tagged(
        r#"
// Unpacked structs with a multi-dimensional member in containers: an
// associative array, a dynamic array, a queue and a local array of structs.
typedef struct { bit [7:0] sc; bit [7:0] sd [2]; bit [7:0] mm [2][2]; } us_t;

module tb;
  us_t ua, uc;
  us_t aa[int];
  us_t da[];
  us_t q[$];

  function automatic us_t mk(input bit [7:0] base);
    us_t r;
    r.mm[1][0] = base;
    r.sd[1] = base + 8'h10;
    return r;
  endfunction

  initial begin
    ua.sc = 8'hA1;
    ua.mm[0][0] = 8'h30; ua.mm[0][1] = 8'h31; ua.mm[1][0] = 8'h32; ua.mm[1][1] = 8'h33;
    aa[5] = ua;
    aa[7] = mk(8'h40);
    $display("T|aa n=%0d 5.mm11=%h 7.mm10=%h 7.sd1=%h", aa.num(), aa[5].mm[1][1],
             aa[7].mm[1][0], aa[7].sd[1]);
    foreach (aa[k]) $display("T|key %0d", k);
    aa[5].mm[0][0] = 8'hEE;
    uc = aa[5];
    $display("T|aa out mm00=%h mm11=%h", uc.mm[0][0], uc.mm[1][1]);
    $display("T|aa p %p", aa);
    da = new[2];
    da[1] = ua;
    $display("T|da mm11=%h", da[1].mm[1][1]);
    q.push_back(ua);
    uc = mk(8'h50);
    q.push_back(uc);
    q.reverse();
    $display("T|q %0d %h %h", q.size(), q[0].mm[1][0], q[1].mm[0][1]);
    begin : blk
      us_t arr[2];
      arr[1].mm[1][1] = 8'hE5;
      arr[0] = ua;
      $display("T|larr %h %h %p", arr[1].mm[1][1], arr[0].mm[0][1], arr[1]);
    end
  end
endmodule
"#,
    );
    assert_eq!(
        o,
        [
            "T|aa n=2 5.mm11=33 7.mm10=40 7.sd1=50",
            "T|key 5",
            "T|key 7",
            "T|aa out mm00=ee mm11=33",
            "T|aa p '{5:'{sc:'ha1, sd:'{'h0, 'h0}, mm:'{'{'hee, 'h31}, '{'h32, 'h33}}}, 7:'{sc:'h0, sd:'{'h0, 'h50}, mm:'{'{'h0, 'h0}, '{'h40, 'h0}}}}",
            "T|da mm11=33",
            "T|q 2 50 31",
            "T|larr e5 31 '{sc:'h0, sd:'{'h0, 'h0}, mm:'{'{'h0, 'h0}, '{'h0, 'he5}}}",
        ],
        "{o:#?}"
    );
}

/// A nonblocking whole-struct assignment updates every member leaf (scalar
/// ones included) in the NBA region, and 2-state array members default to 0.
#[test]
fn nonblocking_whole_struct_and_two_state_defaults() {
    let o = tagged(
        r#"
// Nonblocking whole-struct assignment, element NBAs, and the 2-state
// default of every element of an array member.
typedef struct { bit [7:0] sc; bit [7:0] sd [2]; bit [7:0] mm [2][2]; } us_t;
typedef struct { bit [7:0] sc; int k; } s0_t;

module tb;
  us_t a, b, c;
  s0_t e, f;
  initial begin
    $display("T|dflt %p", c);
    a.sc = 8'h11; a.sd[0] = 8'h22; a.sd[1] = 8'h33;
    a.mm[0][0] = 8'h30; a.mm[0][1] = 8'h31; a.mm[1][0] = 8'h32; a.mm[1][1] = 8'h33;
    e.sc = 8'h44; e.k = 55;
    b <= a;
    f <= e;
    $display("T|before %p", b);
    #1;
    $display("T|nba %p", b);
    $display("T|nba0 %p", f);
    c.sd[0] <= 8'h44;
    c.mm[1][0] <= 8'h55;
    #1;
    $display("T|elem %p", c);
  end
endmodule
"#,
    );
    assert_eq!(
        o,
        [
            "T|dflt '{sc:'h0, sd:'{'h0, 'h0}, mm:'{'{'h0, 'h0}, '{'h0, 'h0}}}",
            "T|before '{sc:'h0, sd:'{'h0, 'h0}, mm:'{'{'h0, 'h0}, '{'h0, 'h0}}}",
            "T|nba '{sc:'h11, sd:'{'h22, 'h33}, mm:'{'{'h30, 'h31}, '{'h32, 'h33}}}",
            "T|nba0 '{sc:'h44, k:55}",
            "T|elem '{sc:'h0, sd:'{'h44, 'h0}, mm:'{'{'h0, 'h0}, '{'h55, 'h0}}}",
        ],
        "{o:#?}"
    );
}

/// Struct input ports (ANSI and non-ANSI) and struct copies inside a
/// sub-module, including a member that is a 2-D array of structs.
#[test]
fn multidim_member_in_submodules() {
    let o = tagged(
        r#"
// Struct input ports (ANSI continuous and non-ANSI procedural) and struct
// copies inside a sub-module, including an array of structs as a member.
typedef struct { bit [7:0] sc; bit [7:0] sd [2]; bit [7:0] mm [2][2]; } us_t;
typedef struct { bit [3:0] x; bit [3:0] y [2]; } in_t;
typedef struct { in_t g [2][2]; int k; } ns_t;

module suba (input us_t pi);
  initial #2 $display("T|suba sc=%h sd1=%h mm10=%h", pi.sc, pi.sd[1], pi.mm[1][0]);
endmodule

module subn (pi);
  input us_t pi;
  us_t loc;
  initial begin
    #2;
    loc = pi;
    $display("T|subn sc=%h sd1=%h mm10=%h loc=%h", pi.sc, pi.sd[1], pi.mm[1][0], loc.mm[1][0]);
    #2;
    $display("T|subn upd %h", pi.mm[1][0]);
  end
endmodule

module holder;
  us_t h1, h2;
  ns_t n1, n2;
  initial begin
    #1;
    h1.mm[1][0] = 8'h5E;
    h1.sd[1] = 8'h5D;
    h2 = h1;
    $display("T|holder mm10=%h sd1=%h", h2.mm[1][0], h2.sd[1]);
    n1.g[1][0].y[1] = 4'h9;
    n1.g[0][1].x = 4'h3;
    n1.k = 77;
    n2 = n1;
    $display("T|nested %h %h %0d", n2.g[1][0].y[1], n2.g[0][1].x, n2.k);
    $display("T|nested p %p", n2);
  end
endmodule

module tb;
  us_t a;
  suba ua (.pi(a));
  subn un (.pi(a));
  holder hh ();
  initial begin
    a.sc = 8'hA1; a.sd[1] = 8'h23; a.mm[1][0] = 8'h32;
    #3;
    a.mm[1][0] = 8'h99;
  end
endmodule
"#,
    );
    assert_eq!(
        o,
        [
            "T|holder mm10=5e sd1=5d",
            "T|nested 9 3 77",
            "T|nested p '{g:'{'{'{x:'h0, y:'{'h0, 'h0}}, '{x:'h3, y:'{'h0, 'h0}}}, '{'{x:'h0, y:'{'h0, 'h9}}, '{x:'h0, y:'{'h0, 'h0}}}}, k:77}",
            "T|suba sc=a1 sd1=23 mm10=32",
            "T|subn sc=a1 sd1=23 mm10=32 loc=32",
            "T|subn upd 99",
        ],
        "{o:#?}"
    );
}
