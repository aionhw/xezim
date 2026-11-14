//! §11.4.5, §11.4.6, §7.4.3 (#1043): the equality operators on unpacked
//! arrays (fixed, multi-dimensional, queue, dynamic, associative, class
//! properties) and unpacked structs fold their elements three-valued. A
//! definitely differing element decides, wherever it sits; otherwise an x/z
//! element leaves the relation open and `==`/`!=` are x ("the result shall be
//! a 1-bit unknown value (x)", §11.4.5), `===`/`!==` compare the 4-state bits
//! exactly, and `==?`/`!=?` treat x/z bits of the right operand as
//! wildcards. The reference simulator resolves an open relation to neither
//! (`==` and `!=` both 0, an associative array's x element to a mismatch);
//! those expectations follow the LRM, every other one is the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
}

#[test]
fn unknown_elements_fold_three_valued() {
    let msgs = lines(
        r#"
module top;
  typedef logic [2:0] u3_t;
  u3_t ua[0:3], uc[0:3], ux1[0:3], ux2[0:3];
  u3_t qa[$], qx1[$], qx2[$];
  u3_t da[], dx1[];
  u3_t aa[int], ax1[int], ax2[int];
  typedef struct { u3_t p; u3_t q; } us_t;
  us_t sa, sx, sx1;
  initial begin
    ua = '{2,2,2,2}; uc = '{2,2,'x,2}; ux1 = '{3,2,'x,2}; ux2 = '{2,2,'x,3};
    qa = '{2,2}; qx1 = '{3,'x}; qx2 = '{'x,3};
    da = new[2]; dx1 = new[2]; da[0]=2; da[1]=2; dx1[0]='x; dx1[1]=3;
    aa[0]=2; aa[1]=2; ax1[0]=3; ax1[1]='x; ax2[0]='x; ax2[1]=3;
    sa = '{2,2}; sx = '{2,'x}; sx1 = '{3,'x};
    $display("F1 %b %b", ua == uc, ua != uc);
    $display("F2 %b %b", ua == ux1, ua != ux1);
    $display("F3 %b %b", ua == ux2, ua != ux2);
    $display("F4 %b %b", uc == uc, uc != uc);
    $display("F5 %b %b", !(ua == uc), !(ua != uc));
    $display("F6 %b %b", ua === uc, ua !== uc);
    $display("P1 %b %b", ua == '{2,2,'x,2}, ua != '{2,2,'x,2});
    $display("P2 %b %b", ua == '{3,2,'x,2}, ua != '{3,2,'x,2});
    $display("P3 %b %b", ua == '{2,2,'x,3}, ua != '{2,2,'x,3});
    $display("Q1 %b %b", qa == qx1, qa != qx1);
    $display("Q2 %b %b", qa == qx2, qa != qx2);
    $display("D1 %b %b", da == dx1, da != dx1);
    $display("A1 %b %b", aa == ax1, aa != ax1);
    $display("A2 %b %b", aa == ax2, aa != ax2);
    $display("S1 %b %b", sa == sx, sa != sx);
    $display("S2 %b %b", sa == sx1, sa != sx1);
    $display("S3 %b %b", sa == '{2,'x}, sa != '{3,'x});
  end
endmodule
"#,
    );
    // The reference simulator's values.
    expect(
        &msgs,
        &[
            "F2 0 1", "F3 0 1", "F6 0 1", "P2 0 1", "P3 0 1", "Q1 0 1", "Q2 0 1", "D1 0 1",
            "A1 0 1", "A2 0 1", "S2 0 1",
        ],
    );
    // §11.4.5: x where the relation is open; the reference simulator
    // prints 0 for both `==` and `!=`.
    expect(
        &msgs,
        &["F1 x x", "F4 x x", "F5 x x", "P1 x x", "S1 x x", "S3 x 1"],
    );
}

#[test]
fn unknown_elements_of_nd_struct_member_and_class_collections() {
    let msgs = lines(
        r#"
module top;
  typedef logic [2:0] u3_t;
  u3_t m1[0:1][0:1], m2[0:1][0:1], m3[0:1][0:1];
  typedef struct { u3_t q[$]; int k; } sq_t;
  sq_t s1, s2;
  class C;
    u3_t fa[0:2];
    u3_t qa[$];
    u3_t aa[int];
  endclass
  C c1, c2;
  initial begin
    m1 = '{'{1,2},'{3,4}}; m2 = '{'{1,2},'{'x,4}}; m3 = '{'{1,3},'{'x,4}};
    $display("M1 %b %b %b %b", m1 == m2, m1 != m2, m1 === m2, m1 !== m2);
    $display("M2 %b %b", m1 == m3, m1 != m3);
    $display("M3 %b %b", m2 == m2, m1 == '{'{1,2},'{'x,4}});
    s1.q = '{1,2}; s1.k = 1; s2.q = '{1,'x}; s2.k = 1;
    $display("SQ1 %b %b", s1 == s2, s1 != s2);
    s2.k = 2;
    $display("SQ2 %b %b", s1 == s2, s1 != s2);
    c1 = new; c2 = new;
    c1.fa = '{1,2,3}; c2.fa = '{1,'x,3};
    c1.qa = '{1,2}; c2.qa = '{'x,2};
    c1.aa[0] = 1; c2.aa[0] = 'x;
    $display("C1 %b %b %b", c1.fa == c2.fa, c1.fa != c2.fa, c1.fa === c2.fa);
    $display("C2 %b %b %b", c1.qa == c2.qa, c1.qa != c2.qa, c1.qa === c2.qa);
    $display("C3 %b %b %b", c1.aa == c2.aa, c1.aa != c2.aa, c1.aa === c2.aa);
    c2.fa = '{1,2,3}; c2.qa = '{1,2};
    $display("C4 %b %b %b %b", c1.fa == c2.fa, c1.fa === c2.fa, c1.qa == c2.qa, c1.qa !== c2.qa);
  end
endmodule
"#,
    );
    // The reference simulator's values.
    expect(&msgs, &["M2 0 1", "SQ2 0 1", "C4 1 1 1 0"]);
    // §11.4.5: x where the relation is open; the reference simulator
    // prints 0 for both `==` and `!=` (0 1 for the associative array, C3).
    expect(
        &msgs,
        &[
            "M1 x x 0 1",
            "M3 x x",
            "SQ1 x x",
            "C1 x x 0",
            "C2 x x 0",
            "C3 x x 0",
        ],
    );
}

#[test]
fn case_and_wildcard_equality_of_unpacked_structs_and_arrays() {
    let msgs = lines(
        r#"
module top;
  typedef logic [2:0] u3_t;
  typedef struct { u3_t a; int b; } us_t;
  typedef struct { us_t in; u3_t [0:1] pk; } un_t;
  typedef u3_t st_t [0:3];
  us_t x, y, z;
  un_t n1, n2;
  st_t a, b;
  initial begin
    x = '{1, 5}; y = '{1, 5}; z = '{2, 5};
    $display("T1 %b %b %b %b", x === y, x !== y, x === z, x !== z);
    y.a = 'x; z.a = 'x;
    $display("T2 %b %b %b", x === y, y === z, y !== z);
    n1 = '{'{1,2}, '{3,4}}; n2 = '{'{1,2}, '{3,4}};
    $display("T3 %b %b %b", n1 == n2, n1 === n2, n1 == '{'{1,2}, '{3,4}});
    n2.pk[1] = 5;
    $display("T4 %b %b %b", n1 == n2, n1 !== n2, n1 != '{'{1,2}, '{3,4}});
    a = '{1,2,3,4}; b = '{1,2,'x,4};
    $display("W1 %b %b", a ==? '{1,2,3,4}, a !=? '{1,2,3,4});
    $display("W2 %b %b", a ==? '{1,2,3,5}, a ==? '{1,2,'x,4});
    $display("W3 %b %b", b ==? '{1,2,3,4}, b ==? '{1,2,3,5});
    $display("V1 %b %b", a ==? b, a !=? b);
    $display("V2 %b %b", b ==? a, b !=? a);
  end
endmodule
"#,
    );
    // The reference simulator's values.
    expect(
        &msgs,
        &[
            "T1 1 0 0 1",
            "T2 0 1 0",
            "T3 1 1 1",
            "T4 0 1 0",
            "W1 1 0",
            "W2 0 1",
            "V1 1 0",
        ],
    );
    // §11.4.5: x where the relation is open; the reference simulator
    // prints 0 for both `==` and `!=`.
    expect(&msgs, &["W3 x 0", "V2 x x"]);
}
