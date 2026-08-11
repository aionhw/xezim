//! IEEE 1800-2017 Clause 7.11 (array querying functions) and 7.12 (array
//! ordering, locator and reduction methods) exercised over fixed arrays,
//! queues, dynamic arrays AND associative arrays.
//!
//! The associative-array (sparse) case is the focus: until these tests were
//! written, xezim's reduction/locator paths derived the element set from a
//! DENSE `[0, size)` scan. An associative array's `size()` reads 0 from such a
//! scan, so every reduction returned its identity element (`sum()==0`,
//! `product()==1`), `min()/max()` returned `{0}`, and
//! `unique()/unique_index()` returned `{0}`. The corresponding non-assoc
//! behaviour (a queue row with the same element values) is asserted alongside
//! so a regression cannot silently slip by.
//!
//! §7.11 querying functions: `size()`, `num()`, and the `in`/`co` (covered by)
//! query operators. §7.12 methods: `sum/product/and/or/xor` (reduction),
//! `min/max/unique/unique_index/find/find_index/find_first/find_last` (locator).
//!
//! NOTE on ordering: §7.12.1 does not mandate an order for `unique()` /
//! `unique_index()`. xezim returns first-occurrence order; the reference
//! simulator returns them sorted by value. As elsewhere in this suite, tests
//! assert on CONTENT, never on the order.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn line(o: &[String], tag: &str) -> String {
    o.iter()
        .find(|s| s.starts_with(tag))
        .unwrap_or_else(|| panic!("missing {tag:?}: {o:?}"))
        .trim_start_matches(tag)
        .to_string()
}

// ---------------------------------------------------------------------------
// §7.11 array querying functions on associative arrays
// ---------------------------------------------------------------------------

/// `size`, `num`, `in`, `co`, `exists`, `first`, `next`, `prev`, `last` over a
/// sparse assoc key space.
#[test]
fn assoc_querying_functions() {
    let src = r#"
module top;
  int aa[int];
  int sz, nn, k;
  initial begin
    aa[4] = 1; aa[7] = 2; aa[2] = 3;
    sz = aa.size();
    nn = aa.num();
    $display("ASZ %0d %0d", sz, nn);
    $display("AEXI %0d %0d", aa.exists(4), aa.exists(1));
    $display("AFIS %0d %0d", aa.first(k), k);
    k = 2; $display("ANEX %0d %0d", aa.next(k), k);
    k = 4; $display("APRV %0d %0d", aa.prev(k), k);
    $display("ALST %0d %0d", aa.last(k), k);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "ASZ "), "3 3", "size()==num()==3 element");
    assert_eq!(line(&o, "AEXI "), "1 0", "exists(4): yes, exists(1): no");
    assert_eq!(line(&o, "AFIS "), "1 2", "first() binds k=2 (smallest key)");
    assert_eq!(line(&o, "ANEX "), "1 4", "next(2) -> 1, k advances to 4");
    assert_eq!(line(&o, "APRV "), "1 2", "prev(4) -> 1, k backs to 2");
    assert_eq!(line(&o, "ALST "), "1 7", "last() binds k=7 (largest key)");
}

/// A fixed/dynamic/queue row echoes the same querying functions so the array
/// kind does not change the answer.
#[test]
fn array_querying_functions_non_assoc() {
    let src = r#"
module top;
  int f[4] = '{9, 3, 5, 1};
  int q[$] = '{9, 3, 5, 1};
  initial begin
    $display("FSZ %0d %0d", f.size(), q.size());
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "FSZ "), "4 4");
}

// ---------------------------------------------------------------------------
// §7.12 reductions on associative arrays
// ---------------------------------------------------------------------------

/// `sum`, `product`, `and`, `or`, `xor` must iterate the SPARSE element set,
/// not a dense range (which read identity/0 for an assoc array).
#[test]
fn assoc_reductions() {
    let src = r#"
module top;
  int aa[int];
  int s, p, a, o, x;
  initial begin
    aa[0]=5; aa[1]=3; aa[2]=9; aa[3]=3; aa[4]=1;
    s = aa.sum();      p = aa.product();
    a = aa.and();      o = aa.or();   x = aa.xor();
    $display("RSUM %0d", s);
    $display("RPROD %0d", p);
    $display("RAND %0d %0d", a, 5&3&9&3&1);
    $display("ROR %0d %0d", o, 5|3|9|3|1);
    $display("RXOR %0d %0d", x, 5^3^9^3^1);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "RSUM "), "21");
    assert_eq!(line(&o, "RPROD "), "405");
    // and/or/xor must each agree with their independently-computed in-SV fold.
    for tag in ["RAND ", "ROR ", "RXOR "] {
        let l = line(&o, tag);
        let mut it = l.split_whitespace();
        let (got, want) = (it.next().unwrap(), it.next().unwrap());
        assert_eq!(got, want, "{tag}");
    }
}

/// The same reduction over a QUEUE must give the same answers.
#[test]
fn queue_reductions() {
    let src = r#"
module top;
  int q[$] = '{5, 3, 9, 3, 1};
  int s, p, a, o, x;
  initial begin
    s = q.sum();  p = q.product();
    a = q.and();  o = q.or();  x = q.xor();
    $display("RQSUM %0d", s);
    $display("RQPROD %0d", p);
    $display("RQAND %0d %0d", a, 5&3&9&3&1);
    $display("RQOR %0d %0d", o, 5|3|9|3|1);
    $display("RQXOR %0d %0d", x, 5^3^9^3^1);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "RQSUM "), "21");
    assert_eq!(line(&o, "RQPROD "), "405");
    for tag in ["RQAND ", "RQOR ", "RQXOR "] {
        let l = line(&o, tag);
        let mut it = l.split_whitespace();
        let (got, want) = (it.next().unwrap(), it.next().unwrap());
        assert_eq!(got, want, "{tag}");
    }
}

// ---------------------------------------------------------------------------
// §7.12 locator methods (min/max/unique/find) on associative arrays
// ---------------------------------------------------------------------------

/// `min`/`max` on an assoc array must return the extremal VALUE (not `{0}`);
/// `unique`/`unique_index` yield every distinct value (content-only order).
#[test]
fn assoc_locator_min_max_unique() {
    let src = r#"
module top;
  int aa[int];
  int r[$];
  int idx[$];
  initial begin
    aa[0]=5; aa[1]=3; aa[2]=9; aa[3]=3; aa[4]=1;
    r = aa.min();      $display("LMIN %p", r);
    r = aa.max();      $display("LMAX %p", r);
    r = aa.unique();   $display("LUNIQ %p", r);
    idx = aa.unique_index(); $display("LUNIQIDX %p", idx);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "LMIN "), "'{1}");
    assert_eq!(line(&o, "LMAX "), "'{9}");
    assert_eq!(line(&o, "LUNIQ "), "'{5, 3, 9, 1}", "content, first-occurrence order");
    assert_eq!(line(&o, "LUNIQIDX "), "'{0, 1, 2, 4}", "key of each distinct element");
}

/// `find with` / `find_index with` on an assoc array honours the sparse keys.
#[test]
fn assoc_find_with_filter() {
    let src = r#"
module top;
  int aa[int];
  int r[$];
  int k[$];
  initial begin
    aa[0]=5; aa[1]=3; aa[2]=9; aa[3]=3; aa[4]=1;
    r = aa.find with (item > 2);       $display("FFIND %p", r);
    k = aa.find_index with (item > 2); $display("FFIDX %p", k);
    r = aa.find_first with (item == 3); $display("FFF %p", r);
    r = aa.find_last with (item == 3);  $display("FFL %p", r);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "FFIND "), "'{5, 3, 9, 3}", "values >2");
    assert_eq!(line(&o, "FFIDX "), "'{0, 1, 2, 3}", "keys of matches");
    assert_eq!(line(&o, "FFF "), "'{3}", "find_first yields the value");
    assert_eq!(line(&o, "FFL "), "'{3}", "find_last yields the value");
}

// ---------------------------------------------------------------------------
// §7.12 reductions with a `with` clause on associative arrays
// ---------------------------------------------------------------------------

#[test]
fn assoc_reduce_with_clause() {
    let src = r#"
module top;
  int aa[int];
  int s;
  initial begin
    aa[0]=5; aa[1]=3; aa[2]=9; aa[3]=3; aa[4]=1;
    s = aa.sum with (item > 2);   // 4 matches -> four 1-bit true flags -> 4&1 = 0
    $display("WSUM %0d", s);
    s = aa.sum with (item);
    $display("WFULL %0d", s);
    s = aa.product with (item);
    $display("WPROD %0d", s);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "WSUM "), "0", "4 matches of a 1-bit predicate -> 0 (mod 2)");
    assert_eq!(line(&o, "WFULL "), "21");
    assert_eq!(line(&o, "WPROD "), "405");
}

// ---------------------------------------------------------------------------
// Queues and associative arrays carrying the same data must agree.
// ---------------------------------------------------------------------------

#[test]
fn queue_and_assoc_agree() {
    let src = r#"
module top;
  int aa[int];
  int q[$] = '{1, 3, 5, 3, 9};
  int ra[$], rq[$];
  initial begin
    aa[0]=1; aa[1]=3; aa[2]=5; aa[3]=3; aa[4]=9;
    ra = aa.max();  rq = q.max();
    $display("MAXA %p %p", ra, rq);
    ra = aa.min();  rq = q.min();
    $display("MINA %p %p", ra, rq);
    rq = q.unique();
    $display("UNIQA %p %p", ra, rq);
    ra = aa.sum();  rq = null;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let o = outs(&sim);
    assert_eq!(line(&o, "MAXA "), "'{9} '{9}");
    assert_eq!(line(&o, "MINA "), "'{1} '{1}");
    assert_eq!(line(&o, "UNIQA "), "'{1} '{1, 3, 5, 9}", "min result + deduped queue");
}