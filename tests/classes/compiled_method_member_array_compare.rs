//! Comparisons between two elements of a class-member fixed-size
//! array inside a BYTECODE-COMPILED method (`base[i] < base[j] + len[j]`).
//!
//! The element read lowers to `LoadCollElem`, but the compiler's width
//! inference (`expr_max_width`) had no table for the element width of a
//! member fixed array — every lookup missed and the select sized at
//! 1 bit. A self-determined context (a comparison) then resized both
//! element reads to their LSBs, so the comparison silently evaluated
//! the wrong values (with the reproducer's all-even data, `0 < 0`).
//! The interpreter path and `$display` argument evaluation were always
//! correct. The fix mirrors the declared element widths of the
//! element-access plan members into the compiler as a plan fact
//! (`coll_elem_widths`) and consults it in `expr_max_width`.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// The reproducer shape: a loop method (compiled tier from the first
/// call) counting single-sided and both-sided overlaps over two member
/// arrays. Region 1 = [8192, +4096) and region 2 = [8192, +2048)
/// overlap; the single-sided relation holds for all 10 (i<j) pairs.
#[test]
fn compiled_method_two_subscript_overlap() {
    const SRC: &str = "class region_list;
  int unsigned base[5];
  int unsigned len[5];
  int hits_single, hits_both;
  function void count_pairs();
    bit both;
    hits_single = 0;
    hits_both = 0;
    for (int i = 0; i < 5; i++)
      for (int j = i + 1; j < 5; j++) begin
        if (base[i] < base[j] + len[j])
          hits_single = hits_single + 1;
        both = (base[i] < base[j] + len[j]) && (base[j] < base[i] + len[i]);
        if (both)
          hits_both = hits_both + 1;
      end
  endfunction
endclass

module tb;
  int hs = 0;
  int hb = 0;
  int m_hits = 0;
  int unsigned m_base[5];
  int unsigned m_len[5];
  initial begin
    region_list r = new;
    r.base = '{0, 8192, 8192, 14336, 15360};
    r.len  = '{8192, 4096, 2048, 1024, 512};
    r.count_pairs();
    hs = r.hits_single;
    hb = r.hits_both;

    // Module-scope control: identical data and loop, no class.
    m_base = '{0, 8192, 8192, 14336, 15360};
    m_len  = '{8192, 4096, 2048, 1024, 512};
    m_hits = 0;
    for (int i = 0; i < 5; i++)
      for (int j = i + 1; j < 5; j++)
        if (m_base[i] < m_base[j] + m_len[j] && m_base[j] < m_base[i] + m_len[i])
          m_hits = m_hits + 1;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "hs"), 10, "single-sided count from compiled method");
    assert_eq!(
        u(&sim, "hb"),
        1,
        "both-sided (overlap) count from compiled method"
    );
    assert_eq!(u(&sim, "m_hits"), 1, "module-scope control count");
}

/// Variation: a different element width (8-bit packed elements),
/// different array size, and an equality comparison alongside `<`.
/// Values {3, 200, 17, 200}: pairs i<j with v[i] < v[j] are
/// (0,1), (0,2), (0,3), (2,3) = 4; pairs with v[i] == v[j]: (1,3) = 1.
/// (At 1-bit sizing the LSBs {1,0,1,0} would give different counts.)
#[test]
fn compiled_method_two_subscript_narrow_elements() {
    const SRC: &str = "class narrow;
  bit [7:0] v[4];
  int lt_cnt, eq_cnt;
  function void count_pairs();
    lt_cnt = 0;
    eq_cnt = 0;
    for (int i = 0; i < 4; i++)
      for (int j = i + 1; j < 4; j++) begin
        if (v[i] < v[j]) lt_cnt = lt_cnt + 1;
        if (v[i] == v[j]) eq_cnt = eq_cnt + 1;
      end
  endfunction
endclass

module tb;
  int lt = 0;
  int eq = 0;
  initial begin
    narrow n = new;
    n.v = '{3, 200, 17, 200};
    n.count_pairs();
    lt = n.lt_cnt;
    eq = n.eq_cnt;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "lt"), 4, "8-bit element < count");
    assert_eq!(u(&sim, "eq"), 1, "8-bit element == count");
}

/// Negative control: genuinely non-overlapping regions must still
/// report no both-sided hits (the fix must not manufacture hits),
/// while the single-sided count stays correct (6 of 6 pairs).
#[test]
fn compiled_method_no_overlap_negative() {
    const SRC: &str = "class disjoint;
  int unsigned base[4];
  int unsigned len[4];
  int hits_single, hits_both;
  function void count_pairs();
    hits_single = 0;
    hits_both = 0;
    for (int i = 0; i < 4; i++)
      for (int j = i + 1; j < 4; j++) begin
        if (base[i] < base[j] + len[j])
          hits_single = hits_single + 1;
        if ((base[i] < base[j] + len[j]) && (base[j] < base[i] + len[i]))
          hits_both = hits_both + 1;
      end
  endfunction
endclass

module tb;
  int hs = 0;
  int hb = 0;
  initial begin
    disjoint d = new;
    d.base = '{0, 100, 200, 300};
    d.len  = '{50, 50, 50, 50};
    d.count_pairs();
    hs = d.hits_single;
    hb = d.hits_both;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "hs"), 6, "single-sided count for disjoint regions");
    assert_eq!(u(&sim, "hb"), 0, "no overlaps among disjoint regions");
}

/// Member QUEUE element selects size from the same plan fact (the
/// producer covers queues as well as fixed arrays). Values
/// {0, 8192, 16384} with lengths {8192, 8192, 8192}: all 3 pairs
/// satisfy the single-sided relation.
#[test]
fn compiled_method_two_subscript_queue_member() {
    const SRC: &str = "class with_queue;
  int unsigned base[$];
  int unsigned len[$];
  int hits_single;
  function void count_pairs();
    hits_single = 0;
    for (int i = 0; i < 3; i++)
      for (int j = i + 1; j < 3; j++)
        if (base[i] < base[j] + len[j])
          hits_single = hits_single + 1;
  endfunction
endclass

module tb;
  int hs = 0;
  initial begin
    with_queue q = new;
    q.base = '{0, 8192, 16384};
    q.len  = '{8192, 8192, 8192};
    q.count_pairs();
    hs = q.hits_single;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "hs"), 3, "queue member element < count");
}

/// A subclass that REDECLARES the member with a wider element type
/// (§8.14: the leaf-most declarer owns the name) must size its selects
/// at the SUBCLASS's element width, not the base class's. Base
/// elements are 8-bit; the subclass's are 32-bit, and the values
/// {70000, 3, 200000} need the full width: over all ordered pairs
/// i != j, v[i] < v[j] holds for (0,2), (1,0) and (1,2) — count 3.
#[test]
fn compiled_method_redeclared_member_width() {
    const SRC: &str = "class base_c;
  bit [7:0] v[3];
endclass
class sub_c extends base_c;
  int unsigned v[3];
  function int count_lt();
    int n = 0;
    for (int i = 0; i < 3; i++)
      for (int j = 0; j < 3; j++)
        if (v[i] < v[j]) n++;
    return n;
  endfunction
endclass

module tb;
  int cnt = 0;
  initial begin
    sub_c s = new;
    s.v = '{70000, 3, 200000};
    cnt = s.count_lt();
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "cnt"),
        3,
        "redeclared member sizes at subclass width"
    );
}
