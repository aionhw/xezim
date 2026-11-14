//! §7.12.2, §7.12.1 (#1059): `sort`/`rsort` with a `with (item.<member>)`
//! clause bind the members of a packed-struct element, as the locators and
//! reductions do, and order by the key's value; `sort`, `rsort`, `min`, `max`
//! and `unique` compare elements and keys at their full width, signed or
//! unsigned as their type is, a real as a real. The members were unbound
//! (every key 0, so the stable sort kept the queue as it was), and a 64-bit
//! key kept the low word of a wider value and read a signed element as
//! unsigned. Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
    assert!(
        !msgs.iter().any(|m| m.starts_with("FAIL")),
        "unexpected FAIL in {msgs:#?}"
    );
}

/// The reporter's program: a bag sorted by a member in `post_randomize`, the
/// leading, trailing and middle member, `rsort`, a named iterator, a class
/// method, 128-bit elements and `min`/`max`, a Boolean key, and controls.
#[test]
fn packed_member_keys_order_the_queue() {
    let msgs = lines(
        r##"
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

typedef struct packed {
  bit [63:0] rank;
  bit [63:0] stamp;
} rec_t;

typedef struct packed {
  bit [63:0] a;
  bit [63:0] b;
  bit [63:0] c;
} tri_t;

class box_c;
  rand bit [63:0] rank;
  rand bit [63:0] stamp;
  function new(bit [63:0] r, bit [63:0] s);
    rank  = r;
    stamp = s;
  endfunction
endclass

class bag_c;
  rand rec_t pick;
  rec_t      bag[$];
  function void post_randomize();
    bag.push_back(pick);
    bag.sort() with (item.rank);
  endfunction
endclass

class bin_c;
  rec_t rows[$];
  function void add(bit [63:0] r, bit [63:0] s);
    rec_t e;
    e = '{rank: r, stamp: s};
    rows.push_back(e);
    rows.sort() with (item.rank);
  endfunction
endclass

module tb_top;

  `SVTEST_INIT

  bit [63:0] RK[6] = '{11, 48, 85, 25, 62, 3};
  bit [63:0] ST[6] = '{7, 60, 24, 77, 41, 52};
  bit [63:0] CC[6] = '{9, 33, 5, 71, 19, 44};

  int    ignore;
  bag_c  bg;
  bin_c  bn;
  rec_t  e,  q[$];
  tri_t  f,  t[$];
  box_c  bx, h[$];
  bit [63:0] q64[$];
  bit [127:0] qw[$];
  int    iq[$];

  task automatic fill;
    q.delete();   t.delete();  h.delete();
    q64.delete(); iq.delete(); qw.delete();
    for (int i = 0; i < 6; i++) begin
      e = '{rank: RK[i], stamp: ST[i]};    q.push_back(e);
      f = '{a: RK[i], b: ST[i], c: CC[i]};  t.push_back(f);
      bx = new(RK[i], ST[i]);
      h.push_back(bx);
      q64.push_back(RK[i]);
      qw.push_back({RK[i], ST[i]});
      iq.push_back(int'(RK[i]));
    end
  endtask

  function automatic bit asc_rank(input rec_t qq[$]);
    bit ok = 1;
    foreach (qq[i])
      if (i > 0 && qq[i-1].rank > qq[i].rank) ok = 0;
    return ok;
  endfunction

  initial begin

    bg = new;
    for (int i = 0; i < 6; i++)
      ignore = bg.randomize() with { pick.rank == RK[i]; pick.stamp == ST[i]; };
    `SVTEST_CHECK(asc_rank(bg.bag),
                  "c1: bag.sort() with (item.rank) in post_randomize() is a no-op")

    fill;
    q.sort() with (item.rank);
    `SVTEST_CHECK(asc_rank(q), "c2: q.sort() with (item.rank) is a no-op")

    fill;
    q.sort() with (item.stamp);
    begin
      bit ok;
      ok = 1;
      foreach (q[i]) if (i > 0 && q[i-1].stamp > q[i].stamp) ok = 0;
      `SVTEST_CHECK(ok, "c3: q.sort() with (item.stamp) is a no-op")
    end

    fill;
    q.rsort() with (item.rank);
    begin
      bit ok;
      ok = 1;
      foreach (q[i]) if (i > 0 && q[i-1].rank < q[i].rank) ok = 0;
      `SVTEST_CHECK(ok, "c4: q.rsort() with (item.rank) is a no-op")
    end

    fill;
    q.sort(rec) with (rec.rank);
    `SVTEST_CHECK(asc_rank(q), "c5: q.sort(rec) with (rec.rank) is a no-op")

    fill;
    t.sort() with (item.a);
    begin
      bit ok;
      ok = 1;
      foreach (t[i]) if (i > 0 && t[i-1].a > t[i].a) ok = 0;
      `SVTEST_CHECK(ok, "c6: t.sort() with (item.a) is a no-op")
    end

    fill;
    t.sort() with (item.b);
    begin
      bit ok;
      ok = 1;
      foreach (t[i]) if (i > 0 && t[i-1].b > t[i].b) ok = 0;
      `SVTEST_CHECK(ok, "c7: t.sort() with (item.b) is a no-op")
    end

    bn = new;
    for (int i = 0; i < 6; i++) bn.add(RK[i], ST[i]);
    `SVTEST_CHECK(asc_rank(bn.rows),
                  "c8: rows.sort() with (item.rank) inside a class method is a no-op")

    fill;
    q.sort();
    `SVTEST_CHECK(asc_rank(q),
                  "c9: q.sort() on 128-bit elements ordered by the low 64 bits only")

    fill;
    qw.sort();
    begin
      bit ok;
      ok = 1;
      foreach (qw[i]) if (i > 0 && qw[i-1] > qw[i]) ok = 0;
      `SVTEST_CHECK(ok, "c10: qw.sort() on 128-bit elements ordered by the low 64 bits only")
    end

    fill;
    begin
      rec_t loq[$], hiq[$];
      bit ok;
      loq = q.min();
      hiq = q.max();
      ok = (loq.size() == 1) && (hiq.size() == 1) &&
           (loq[0].rank == 64'd3)  && (loq[0].stamp == 64'd52) &&
           (hiq[0].rank == 64'd85) && (hiq[0].stamp == 64'd24);
      `SVTEST_CHECK(ok, "c11: q.min()/q.max() on 128-bit elements picked on the low 64 bits only")
    end

    fill;
    h.sort() with (item.rank);
    begin
      bit ok;
      ok = 1;
      foreach (h[i]) if (i > 0 && h[i-1].rank > h[i].rank) ok = 0;
      `SVTEST_CHECK(ok, "k1 control: class-handle queue sort must be ascending")
    end

    fill;
    iq.sort();
    begin
      bit ok;
      ok = 1;
      foreach (iq[i]) if (i > 0 && iq[i-1] > iq[i]) ok = 0;
      `SVTEST_CHECK(ok, "k2 control: int queue plain sort() must be ascending")
    end

    fill;
    iq.sort() with (item);
    begin
      bit ok;
      ok = 1;
      foreach (iq[i]) if (i > 0 && iq[i-1] > iq[i]) ok = 0;
      `SVTEST_CHECK(ok, "k3 control: int queue sort() with (item) must be ascending")
    end

    fill;
    begin
      rec_t sel[$];
      sel = q.find() with (item.rank == 25);
      `SVTEST_CHECK(sel.size() == 1 && sel[0].stamp == 77,
                    "k4 control: q.find() with (item.rank) must pick the rank-25 record")
    end

    fill;
    `SVTEST_CHECK(q.sum() with (longint'(item.rank)) == 234,
                  "k5 control: q.sum() with (item.rank) must be 234")

    fill;
    begin
      rec_t uniq[$];
      uniq = q.unique() with (item.rank);
      `SVTEST_CHECK(uniq.size() == 6,
                    "k6 control: q.unique() with (item.rank) must keep all 6 records")
    end

    fill;
    q64.sort();
    begin
      bit ok;
      ok = 1;
      foreach (q64[i]) if (i > 0 && q64[i-1] > q64[i]) ok = 0;
      `SVTEST_CHECK(ok, "k7 control: 64-bit queue plain sort() must be ascending")
    end

    fill;
    begin
      bit ok;
      ok = (q.size() == 6);
      for (int i = 0; i < 6; i++)
        if (q[i].rank != RK[i] || q[i].stamp != ST[i]) ok = 0;
      `SVTEST_CHECK(ok, "k8 control: fill() must leave the six records in push order")
    end

    fill;
    begin
      rec_t loq[$];
      bit ok;
      loq = q.min() with (item.rank);
      ok = (loq.size() == 1) && (loq[0].rank == 64'd3) && (loq[0].stamp == 64'd52);
      `SVTEST_CHECK(ok, "k9 control: q.min() with (item.rank) must pick the rank-3 record")
    end

    fill;
    begin
      rec_t hiq[$];
      bit ok;
      hiq = q.max() with (item.stamp);
      ok = (hiq.size() == 1) && (hiq[0].rank == 64'd25) && (hiq[0].stamp == 64'd77);
      `SVTEST_CHECK(ok, "k10 control: q.max() with (item.stamp) must pick the stamp-77 record")
    end

    fill;
    q.sort() with (item.rank > 50);
    begin
      bit ok;
      ok = 1;
      for (int i = 0; i < 4; i++) if ( q[i].rank > 50) ok = 0;
      for (int i = 4; i < 6; i++) if (q[i].rank <= 50) ok = 0;
      `SVTEST_CHECK(ok, "c12: q.sort() with (item.rank > 50) did not partition the queue")
    end

    `SVTEST_PASSFAIL
  end

endmodule
"##,
    );
    expect(&msgs, &["TEST_PASS"]);
}

/// Signed, unsigned, real, signed-member, wide-member and 200-bit elements
/// through sort, rsort, sum, min, max and unique.
#[test]
fn elements_order_by_their_full_value() {
    let msgs = lines(
        r##"
module top;
  typedef struct packed { bit [3:0] hi; bit signed [3:0] lo; } nib_t;
  typedef struct packed { bit [99:0] big; bit [27:0] tag; } wide_t;
  int        iq[$]  = '{5, -3, 0, 7, -10};
  shortint   sq[$]  = '{-1, 2, -300, 4};
  bit [7:0]  bq[$]  = '{8'hff, 8'h01, 8'h80};
  real       rq[$]  = '{2.5, -1.0, 0.25, -7.5};
  nib_t      nq[$];
  wide_t     wq[$];
  bit [199:0] vq[$];
  initial begin
    nq.push_back('{4'd1, -4'sd2}); nq.push_back('{4'd9, 4'sd3}); nq.push_back('{4'd3, -4'sd8});
    wq.push_back('{100'h1, 28'd3}); wq.push_back('{{4'h8, 96'h0}, 28'd1}); wq.push_back('{100'h0, 28'd9});
    vq.push_back({8'h01, 192'h0}); vq.push_back(200'h5); vq.push_back({8'h80, 192'h0});
    iq.sort();  $display("int sort   %0d %0d %0d %0d %0d", iq[0], iq[1], iq[2], iq[3], iq[4]);
    iq.rsort(); $display("int rsort  %0d %0d %0d %0d %0d", iq[0], iq[1], iq[2], iq[3], iq[4]);
    sq.sort();  $display("short sort %0d %0d %0d %0d", sq[0], sq[1], sq[2], sq[3]);
    bq.sort();  $display("byte sort  %0d %0d %0d", bq[0], bq[1], bq[2]);
    rq.sort();  $display("real sort  %0.2f %0.2f %0.2f %0.2f", rq[0], rq[1], rq[2], rq[3]);
    rq.rsort(); $display("real rsort %0.2f %0.2f %0.2f %0.2f", rq[0], rq[1], rq[2], rq[3]);
    nq.sort() with (item.lo);  $display("nib lo     %0d %0d %0d", nq[0].lo, nq[1].lo, nq[2].lo);
    nq.rsort() with (item.hi); $display("nib hi r   %0d %0d %0d", nq[0].hi, nq[1].hi, nq[2].hi);
    $display("nib sum lo %0d", nq.sum() with (int'(item.lo)));
    wq.sort() with (item.big); $display("wide big   %0d %0d %0d", wq[0].tag, wq[1].tag, wq[2].tag);
    wq.rsort(); $display("wide whole r %0d %0d %0d", wq[0].tag, wq[1].tag, wq[2].tag);
    vq.sort(); $display("200b sort  %0h %0h %0h", vq[0][199:192], vq[1][7:0], vq[2][199:192]);
    begin
      int mn[$], mx[$]; nib_t nm[$]; wide_t wm[$]; bit [199:0] vu[$]; real rm[$];
      iq = '{5, -3, 0, 7, -10};
      mn = iq.min(); mx = iq.max();
      $display("int min/max %0d %0d", mn[0], mx[0]);
      rm = rq.min(); $display("real min %0.2f", rm[0]);
      nm = nq.min() with (item.lo); $display("nib min lo %0d", nm[0].lo);
      nm = nq.max() with (item.lo); $display("nib max lo %0d", nm[0].lo);
      wm = wq.max() with (item.big); $display("wide max big tag %0d", wm[0].tag);
      vq.push_back(200'h5);
      vu = vq.unique(); $display("200b unique size %0d", vu.size());
    end
    $finish;
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "int sort   -10 -3 0 5 7",
            "int rsort  7 5 0 -3 -10",
            "short sort -300 -1 2 4",
            "byte sort  1 128 255",
            "real sort  -7.50 -1.00 0.25 2.50",
            "real rsort 2.50 0.25 -1.00 -7.50",
            "nib lo     -8 -2 3",
            "nib hi r   9 3 1",
            "nib sum lo -7",
            "wide big   9 3 1",
            "wide whole r 1 3 9",
            "200b sort  0 0 80",
            "int min/max -10 7",
            "real min -7.50",
            "nib min lo -8",
            "nib max lo 3",
            "wide max big tag 1",
            "200b unique size 3",
        ],
    );
}

/// §10.9, §7.10.2: an assignment pattern pushed into a queue of packed
/// structs (`push_back`, `push_front`, `insert`) takes the element type, as
/// one assigned to a variable of it does: each item is sized to its member.
#[test]
fn pushed_patterns_take_the_packed_element_type() {
    let msgs = lines(
        r##"
module top;
  typedef struct packed { bit [15:0] k; bit [15:0] v; } r_t;
  r_t rq[$];
  r_t e;
  int j = 3;
  initial begin
    rq.push_back('{5, 6});
    rq.push_back('{j * 2, j});
    e = '{j * 2, j}; rq.push_back(e);
    rq.push_front('{7, 8});
    rq.insert(1, '{9, 10});
    foreach (rq[i]) $display("%0d: k=%0d v=%0d", i, rq[i].k, rq[i].v);
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "0: k=7 v=8",
            "1: k=9 v=10",
            "2: k=5 v=6",
            "3: k=6 v=3",
            "4: k=6 v=3",
        ],
    );
}
