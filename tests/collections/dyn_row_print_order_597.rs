//! IEEE 1800-2017 §21.2.1.7, §10.9.1: `%p` prints a fixed row of a dynamic
//! array or queue from its declared left bound, as it prints a fixed array:
//! `int a[][1:0]` printed `'{'{10, 20}}` where `int f[1:0]` prints
//! `'{20, 10}` (#597, reported by akashlevy; both now print from the left
//! bound). Also: a class member shadows a same-named module array in its
//! methods (§8.10), so `a[0][1] = 20` there wrote the module's array; and
//! `push_back('{1, 2, 3})` fills a `[3:1]` row from its left bound.
//!
use xezim::simulate;

fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

#[test]
fn reported_testbench() {
    let src = r#"// §21.2.1.7: %p prints an unpacked array from its left bound to its right
// bound; a row declared [1:0] prints index 1 first.
class C;
  int a[][1:0];
  function void run();
    a = new[1];
    a[0][1] = 20;
    a[0][0] = 10;
    $display("cls %p expect '{'{20, 10}}", a);
  endfunction
endclass
module t;
  int a[][1:0];
  int f[1:0];
  initial begin
    C c = new;
    c.run();
    a = new[1];
    a[0][1] = 20;
    a[0][0] = 10;
    f[1] = 20;
    f[0] = 10;
    $display("mod %p expect '{'{20, 10}}", a);
    $display("fixed %p expect '{20, 10}", f);
  end
endmodule

"#;
    assert_eq!(
        shown(src, &["cls ", "mod ", "fixed "]),
        [
            "cls '{'{20, 10}} expect '{'{20, 10}}",
            "mod '{'{20, 10}} expect '{'{20, 10}}",
            "fixed '{20, 10} expect '{20, 10}"
        ]
    );
}

#[test]
fn rows_print_from_the_left_bound_and_members_shadow() {
    let src = r#"class C;
  int a[][1:0];
  int n[][1:0][0:1];
  function void run();
    a = new[1]; a[0][1] = 20; a[0][0] = 10;
    n = new[1]; n[0][1][0] = 1; n[0][1][1] = 2; n[0][0][0] = 3; n[0][0][1] = 4;
    $display("R %0d %0d %0d %0d", a[0][1], a[0][0], n[0][1][1], n[0][0][0]);
    $display("P %p %p", a, n);
  endfunction
endclass
module t;
  int a[][1:0];
  int n[][1:0][0:1];
  int z[$][3:1];
  initial begin
    C c = new; c.run();
    a = new[1]; a[0][1] = 7;
    n = new[1]; n[0][1][1] = 5;
    z.push_back('{1, 2, 3});
    $display("M %0d %p %p %p", a[0][1], a, n, z);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["R ", "P ", "M "]),
        [
            "R 20 10 2 3",
            "P '{'{20, 10}} '{'{'{1, 2}, '{3, 4}}}",
            "M 7 '{'{7, 0}} '{'{'{0, 5}, '{0, 0}}} '{'{1, 2, 3}}"
        ]
    );
}

#[test]
fn push_unpacked_concatenation_into_fixed_row() {
    // §7.10.2.6, §10.10: the queue element supplies the unpacked array context.
    let src = r#"module t;
  typedef int pair_t [2];
  pair_t pairs[$], e;
  int rows[$][3:1];
  initial begin
    pairs.push_back('{1, 2});
    pairs.push_back({3, 4});
    e = '{5, 6};
    pairs.push_back(e);
    rows.push_back({e, 7});
    $display("pairs %0d %p", pairs.size(), pairs);
    $display("row %0d %0d %0d", rows[0][3], rows[0][2], rows[0][1]);
  end
endmodule"#;
    assert_eq!(
        shown(src, &["pairs ", "row "]),
        ["pairs 3 '{'{1, 2}, '{3, 4}, '{5, 6}}", "row 5 6 7"]
    );
}

/// IEEE 1800-2023 §7.6, §10.9.1, §10.10 (#1467).
#[test]
fn whole_row_writes_through_a_class_handle() {
    let src = r#"module tb;
  typedef int pair_t[2];
  class K;
    pair_t q[$], d[];
    int f[2][2];
  endclass
  K k; pair_t p;
  int i;
  initial begin
    k = new; p = '{5, 6};
    repeat (3) k.q.push_back('{0, 0});
    k.d = new[2];
    k.q[i++] = '{1, 2}; k.q[i++] = {3, 4}; k.q[i++] = p;
    k.d[0] = '{7, 8}; k.d[1] = {9, 10};
    k.f[0] = '{1, 2}; k.f[1] = {3, 4};
    $display("Q %p D %p F %p i=%0d", k.q, k.d, k.f, i);
    k.d[2] = '{3, 4}; k.q[$+1] = '{default:11};
    $display("size %0d %0d append %p", k.d.size(), k.q.size(), k.q);
  end
endmodule"#;
    assert_eq!(shown(src, &["Q ", "size "]), [
        "Q '{'{1, 2}, '{3, 4}, '{5, 6}} D '{'{7, 8}, '{9, 10}} F '{'{1, 2}, '{3, 4}} i=3",
        "size 2 4 append '{'{1, 2}, '{3, 4}, '{5, 6}, '{11, 11}}",
    ]);
}
