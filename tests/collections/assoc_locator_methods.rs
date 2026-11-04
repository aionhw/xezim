//! The locator methods search an associative array in index order, and
//! the `_index` forms return its keys (§7.12.1, §7.12.4, §7.8). Only the
//! positions `0..size` of a queue were walked, so on an associative array
//! every locator found nothing: the axi4 AVIP scoreboard's
//! `find_first_index() with (item.arid == t1.rid)` over its address table
//! came back empty and every read was checked against entry 0.
//!
//! Expected values are the reference simulator's.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const SRC_HANDLES: &str = r#"
typedef enum bit [3:0] {A0, A1, A2} a_e;
typedef enum bit [3:0] {R0, R1, R2} r_e;
class m; a_e id; int v; endclass
class t; r_e rid; a_e aid; int k; endclass
module top; initial begin
  m aq[int]; m qq[$]; t x = new(); int r[$]; a_e loc; r_e rloc;
  for (int i = 0; i < 3; i++) begin automatic m o = new(); o.id = a_e'(i); o.v = i; aq[i] = o; qq.push_back(o); end
  x.rid = R2; x.aid = A2; x.k = 2; loc = A2; rloc = R2;
  r = aq.find_first_index() with (item.id == x.rid); $display("T|a1 %p", r);
  r = aq.find_first_index() with (item.id == x.aid); $display("T|a2 %p", r);
  r = aq.find_first_index() with (item.v == x.k); $display("T|a3 %p", r);
  r = aq.find_first_index() with (item.id == loc); $display("T|a4 %p", r);
  r = aq.find_first_index() with (item.id == rloc); $display("T|a5 %p", r);
  r = qq.find_first_index() with (item.id == x.rid); $display("T|q1 %p", r);
  r = qq.find_first_index() with (item.v == x.k); $display("T|q2 %p", r);
  begin int ai[int]; ai[5] = 7; ai[9] = 2; r = ai.find_first_index() with (item == 2); $display("T|i1 %p", r);
    r = ai.find_index() with (item > 1); $display("T|i2 %p", r); end
  r = aq.find_index() with (item.v >= 1); $display("T|a6 %p", r);
  r = aq.find_last_index() with (item.v >= 1); $display("T|a7 %p", r);
  begin m mm[$]; mm = aq.find_first() with (item.v == 1); $display("T|a8 %0d %0d", mm.size(), mm.size() ? mm[0].v : -1); end
end endmodule
"#;

/// Associative arrays of class handles, `with` clauses naming another
/// object's members, locals of other enum types, and the queue twin.
#[test]
fn assoc_locators_over_class_handles() {
    assert_eq!(
        t_lines(SRC_HANDLES),
        [
            "T|a1 '{2}",
            "T|a2 '{2}",
            "T|a3 '{2}",
            "T|a4 '{2}",
            "T|a5 '{2}",
            "T|q1 '{2}",
            "T|q2 '{2}",
            "T|i1 '{9}",
            "T|i2 '{5, 9}",
            "T|a6 '{1, 2}",
            "T|a7 '{2}",
            "T|a8 1 1",
        ]
    );
}

const SRC_INTEGRAL: &str = r#"
module top; initial begin
  int ai[int]; byte bi[bit [7:0]]; int sa[string]; int r[$]; string rs[$]; int v[$];
  ai[3] = 30; ai[1] = 10; ai[7] = 70; bi[8'd200] = 5; bi[8'd4] = 6;
  sa["b"] = 2; sa["a"] = 1;
  r = ai.find_index(x) with (x > 15); $display("T|1 %p", r);
  r = ai.find_last_index() with (item.index < 7); $display("T|2 %p", r);
  v = ai.find() with (item >= 10); $display("T|3 %p", v);
  $display("T|4 %p", ai.find_first() with (item > 20));
  v = ai.min(); $display("T|5 %p", v);
  begin bit [7:0] rb[$]; rb = bi.find_index() with (item == 5); $display("T|6 %p", rb); end
  r = ai.unique_index(); $display("T|8 %p", r);
end endmodule
"#;

/// Integral keys: a custom iterator, `item.index`, element forms, `%p`
/// of a locator, `min` and `unique_index`.
#[test]
fn assoc_locators_over_integral_keys() {
    assert_eq!(
        t_lines(SRC_INTEGRAL),
        [
            "T|1 '{3, 7}",
            "T|2 '{3}",
            "T|3 '{10, 30, 70}",
            "T|4 '{30}",
            "T|5 '{10}",
            "T|6 '{'hc8}",
            "T|8 '{1, 3, 7}",
        ]
    );
}

const SRC_SCOREBOARD: &str = r#"
typedef enum bit [3:0] {ARID_0, ARID_1, ARID_2} arid_e;
typedef enum bit [3:0] {RID_0, RID_1, RID_2} rid_e;
typedef enum bit [1:0] {READ_FIXED, READ_INCR, READ_WRAP} arburst_e;
class mtx; arid_e arid; arburst_e arburst; int araddr; endclass
class stx; arid_e arid; rid_e rid; endclass
class sb;
  mtx mq[int]; stx sq[int]; int c3, c4;
  function void push(arid_e id, int addr);
    mtx m = new(); stx s = new(); m.arid = id; m.arburst = READ_INCR; m.araddr = addr; s.arid = id;
    mq[c3++] = m; sq[c4++] = s;
  endfunction
  function void check(stx t1);
    int index; int indextemp[$]; mtx a;
    indextemp = sq.find_first_index() with (item.arid == t1.rid);
    index = indextemp[0];
    a = mq[index];
    $display("T|idx=%0d n=%0d addr=%0d burst=%0d", index, indextemp.size(), a.araddr, a.arburst);
    case (a.arburst)
      2'b00: $display("T|case fixed");
      2'b01: $display("T|case incr");
      default: $display("T|case other");
    endcase
    mq.delete(index); sq.delete(index);
  endfunction
endclass
module top; initial begin
  sb s = new(); stx t = new();
  s.push(ARID_1, 10); s.push(ARID_2, 20); s.push(ARID_1, 30);
  t.rid = RID_2; s.check(t); t.rid = RID_1; s.check(t); t.rid = RID_1; s.check(t);
end endmodule
"#;

/// The axi4 AVIP scoreboard's lookup: the first entry whose id matches,
/// then its burst type picks the check.
#[test]
fn assoc_find_first_index_scoreboard_shape() {
    assert_eq!(
        t_lines(SRC_SCOREBOARD),
        [
            "T|idx=1 n=1 addr=20 burst=1",
            "T|case incr",
            "T|idx=0 n=1 addr=10 burst=1",
            "T|case incr",
            "T|idx=2 n=1 addr=30 burst=1",
            "T|case incr",
        ]
    );
}
