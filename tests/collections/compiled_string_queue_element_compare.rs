//! IEEE 1800-2017 §6.16 / §7.10: an element of a LOCAL string queue is a
//! string. In a compiled method body (`XEZIM_METHOD_TIER=0`) the element read
//! took the nominal width of the comparison context — one bit for
//! `q[0] == b` with a string formal `b` — so the compare was always 0, and
//! `q[0] == q[1]` compared bit 0 of each. A string element now keeps its
//! text and compares as a string. Expected values are the reference
//! simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const ISSUE: &str = r#"
class C;
  function int a4(string a, string b); string q[$]; q.push_back(a); return q[0] == b; endfunction
  function int g1(string a); string q[$]; q.push_back(a); return q[0] == "e"; endfunction
  function int g4(string a); string q[$]; q.push_back(a); return q[0] == "ae"; endfunction
  function int g5(string a); string q[$]; q.push_back(a); return q[0] < "f"; endfunction
endclass
module top;
  initial begin
    C c; c = new;
    repeat (2) $display("T|%0d %0d %0d %0d %0d | %0d %0d %0d %0d %0d %0d", c.a4("[0]", "e"), c.a4("ab", "e"), c.a4("xe", "e"), c.a4("e", "e"), c.a4("ab", "ab"),
      c.g1("e"), c.g1("ae"), c.g4("ae"), c.g4("e"), c.g5("e"), c.g5("g"));
  end
endmodule
"#;

#[test]
fn local_string_queue_element_compares_as_string() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let want = ["T|0 0 0 1 1 | 1 0 1 0 1 0", "T|0 0 0 1 1 | 1 0 1 0 1 0"];
    assert_eq!(t_lines(ISSUE), want);
}

const SHAPES: &str = r#"
class C;
  string mq[$];
  function int f1(string a, string b); string q[$]; q.push_back(a); return q[0] == b; endfunction
  function int f2(string a, string b); string q[$]; q.push_back(a); return q[0] == "e"; endfunction
  function int f3(string a, string b); string q[$]; string t; t = b; q.push_back(a); return q[0] == t; endfunction
  function int f4(string a, string b); string q[$]; q.push_back(a); return q[0] != b; endfunction
  function int f5(string a, string b); string d[]; d = new[1]; d[0] = a; return d[0] == b; endfunction
  function int f6(string a, string b); mq.delete(); mq.push_back(a); return mq[0] == b; endfunction
  function int f7(string a, string b); string q[$]; q.push_back(a); return b == q[0]; endfunction
  function int f8(string a, string b); string q[$]; q.push_back(a); return q[0] < b; endfunction
  function int f9(string a, string b); string aa[string]; aa["k"] = a; return aa["k"] == b; endfunction
  function int f10(string a, string b); string q[$]; q.push_back(a); q.push_back(b); return q[0] == q[1]; endfunction
  function int f11(string a, string b); string q[$]; q.push_back(a); return (q[0] == b) ? 5 : 6; endfunction
  function int f12(string a, string b); string q[$]; q.push_back(a); if (q[0] == b) return 1; return 0; endfunction
  function int f13(string a, string b); string q[$]; q.push_back(a); return q[0].compare(b); endfunction
  function int f14(string a, string b); string q[$]; q.push_back(a); return q[0] inside {b}; endfunction
endclass
module top;
  initial begin
    C c; c = new;
    repeat (2) begin
      $display("T|f1 %0d %0d %0d %0d %0d", c.f1("[0]", "e"), c.f1("ab", "e"), c.f1("xe", "e"), c.f1("e", "e"), c.f1("ab", "ab"));
      $display("T|f2 %0d %0d", c.f2("e", "e"), c.f2("ab", "e"));
      $display("T|f3 %0d %0d %0d", c.f3("e", "e"), c.f3("ab", "ab"), c.f3("xe", "e"));
      $display("T|f4 %0d %0d %0d", c.f4("e", "e"), c.f4("ab", "ab"), c.f4("xe", "e"));
      $display("T|f5 %0d %0d %0d", c.f5("e", "e"), c.f5("ab", "ab"), c.f5("xe", "e"));
      $display("T|f6 %0d %0d %0d", c.f6("e", "e"), c.f6("ab", "ab"), c.f6("xe", "e"));
      $display("T|f7 %0d %0d %0d", c.f7("e", "e"), c.f7("ab", "ab"), c.f7("xe", "e"));
      $display("T|f8 %0d %0d %0d", c.f8("e", "f"), c.f8("ab", "ab"), c.f8("b", "ab"));
      $display("T|f9 %0d %0d %0d", c.f9("e", "e"), c.f9("ab", "ab"), c.f9("xe", "e"));
      $display("T|f10 %0d %0d %0d", c.f10("e", "e"), c.f10("ab", "ab"), c.f10("xe", "e"));
      $display("T|f11 %0d %0d %0d", c.f11("e", "e"), c.f11("ab", "ab"), c.f11("xe", "e"));
      $display("T|f12 %0d %0d %0d", c.f12("e", "e"), c.f12("ab", "ab"), c.f12("xe", "e"));
      $display("T|f13 %0d %0d %0d", c.f13("e", "e"), c.f13("ab", "ac"), c.f13("xe", "e"));
      $display("T|f14 %0d %0d %0d", c.f14("e", "e"), c.f14("ab", "ab"), c.f14("xe", "e"));
    end
  end
endmodule
"#;

/// Neighbours: `!=`, `<`, reversed operands, two elements, `if`, `?:`,
/// `compare()`, dynamic-array / member / associative elements.
#[test]
fn string_collection_element_compare_shapes() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let want = [
        "T|f1 0 0 0 1 1",
        "T|f2 1 0",
        "T|f3 1 1 0",
        "T|f4 0 0 1",
        "T|f5 1 1 0",
        "T|f6 1 1 0",
        "T|f7 1 1 0",
        "T|f8 1 0 0",
        "T|f9 1 1 0",
        "T|f10 1 1 0",
        "T|f11 5 5 6",
        "T|f12 1 1 0",
        "T|f13 0 -1 19",
        "T|f14 1 1 0",
        "T|f1 0 0 0 1 1",
        "T|f2 1 0",
        "T|f3 1 1 0",
        "T|f4 0 0 1",
        "T|f5 1 1 0",
        "T|f6 1 1 0",
        "T|f7 1 1 0",
        "T|f8 1 0 0",
        "T|f9 1 1 0",
        "T|f10 1 1 0",
        "T|f11 5 5 6",
        "T|f12 1 1 0",
        "T|f13 0 -1 19",
        "T|f14 1 1 0",
    ];
    assert_eq!(t_lines(SHAPES), want);
}

const TYPEDEF_SHAPES: &str = r#"typedef string sq_t[$];
typedef string sd_t[];
typedef string sa_t[2];
class C;
  sq_t mq;
  function int f1(string a, string b); sq_t q; q.push_back(a); return q[0] == b; endfunction
  function int f2(string a, string b); sq_t q; q.push_back(a); q.push_back(b); return q[0] == q[1]; endfunction
  function int f3(string a, string b); sq_t q; q.push_back(a); return q[0] != b; endfunction
  function int f4(string a, string b); sq_t q; q.push_back(a); return b == q[0]; endfunction
  function int f5(string a, string b); sq_t q; q.push_back(a); return q[0] < b; endfunction
  function int f6(string a, string b); sd_t d; d = new[1]; d[0] = a; return d[0] == b; endfunction
  function int f7(string a, string b); sa_t s; s[1] = a; return s[1] == b; endfunction
  function int f8(string a, string b); mq.delete(); mq.push_back(a); return mq[0] == b; endfunction
  function int f9(string a, string b); sq_t q; q = '{a, b}; return q[1] == b && q[0] == a; endfunction
  function int f10(string a, string b); sq_t q; q.push_back(a); return q[0].len() + (q[0] == "e"); endfunction
  function int f11(string a, string b); sq_t q; q.push_back(a); return q[0] inside {b}; endfunction
  function string f12(string a, string b); sq_t q; q.push_back(a); q.push_back(b); return {q[1], q[0]}; endfunction
endclass
module top;
  initial begin
    C c; c = new;
    repeat (2) begin
      $display("T|f1 %0d %0d %0d %0d", c.f1("e", "e"), c.f1("ab", "ab"), c.f1("xe", "e"), c.f1("[0]", "e"));
      $display("T|f2 %0d %0d %0d", c.f2("e", "e"), c.f2("ab", "ab"), c.f2("xe", "e"));
      $display("T|f3 %0d %0d %0d", c.f3("e", "e"), c.f3("ab", "ab"), c.f3("xe", "e"));
      $display("T|f4 %0d %0d %0d", c.f4("e", "e"), c.f4("ab", "ab"), c.f4("xe", "e"));
      $display("T|f5 %0d %0d %0d", c.f5("e", "f"), c.f5("ab", "ab"), c.f5("b", "ab"));
      $display("T|f6 %0d %0d %0d", c.f6("e", "e"), c.f6("ab", "ab"), c.f6("xe", "e"));
      $display("T|f7 %0d %0d %0d", c.f7("e", "e"), c.f7("ab", "ab"), c.f7("xe", "e"));
      $display("T|f8 %0d %0d %0d", c.f8("e", "e"), c.f8("ab", "ab"), c.f8("xe", "e"));
      $display("T|f9 %0d %0d", c.f9("e", "f"), c.f9("ab", "ab"));
      $display("T|f10 %0d %0d", c.f10("e", "e"), c.f10("abc", "ab"));
      $display("T|f11 %0d %0d", c.f11("e", "e"), c.f11("xe", "e"));
      $display("T|f12 %s", c.f12("ab", "cd"));
    end
  end
endmodule
"#;

/// §6.16 / §6.18: a local declared through a typedef of a string
/// collection (`typedef string sq_t[$]; sq_t q;`) has string elements too;
/// the compiled body compared them at the nominal one-bit width, as it did
/// for `string q[$]`.
#[test]
fn typedef_string_collection_element_compares_as_string() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let want = [
        "T|f1 1 1 0 0",
        "T|f2 1 1 0",
        "T|f3 0 0 1",
        "T|f4 1 1 0",
        "T|f5 1 0 0",
        "T|f6 1 1 0",
        "T|f7 1 1 0",
        "T|f8 1 1 0",
        "T|f9 1 1",
        "T|f10 2 3",
        "T|f11 1 0",
        "T|f12 cdab",
        "T|f1 1 1 0 0",
        "T|f2 1 1 0",
        "T|f3 0 0 1",
        "T|f4 1 1 0",
        "T|f5 1 0 0",
        "T|f6 1 1 0",
        "T|f7 1 1 0",
        "T|f8 1 1 0",
        "T|f9 1 1",
        "T|f10 2 3",
        "T|f11 1 0",
        "T|f12 cdab",
    ];
    assert_eq!(t_lines(TYPEDEF_SHAPES), want);
}

const TYPEDEF_SCOPES: &str = r#"typedef string gq_t[$];
typedef string gd_t[];
typedef string ga_t[string];
typedef gq_t gq2_t;
package pk; typedef string pq_t[$]; endpackage
class C;
  typedef string cq_t[$];
  function int a1(string a, string b); gq_t q; q.push_back(a); return q[0] == b; endfunction
  function int a2(string a); gq_t q; q.push_back(a); return q[0] == "e"; endfunction
  function int a3(string a, string b); gq_t q; q.push_back(a); return q[0] < b; endfunction
  function int a4(string a, string b); gq_t q; q.push_back(a); q.push_back(b); return q[0] == q[1]; endfunction
  function int a5(string a, string b); gq_t q; q.push_back(a); return q[0] != b; endfunction
  function int a7(string a, string b); gq_t q; q.push_back(a); return b == q[0]; endfunction
  function int d1(string a, string b); gd_t d; d = new[1]; d[0] = a; return d[0] == b; endfunction
  function int d2(string a); gd_t d; d = new[1]; d[0] = a; return d[0] == "e"; endfunction
  function int s1(string a, string b); ga_t m; m["k"] = a; return m["k"] == b; endfunction
  function int s2(string a); ga_t m; m["k"] = a; return m["k"] == "e"; endfunction
  function int p1(string a, string b); pk::pq_t q; q.push_back(a); return q[0] == b; endfunction
  function int p2(string a); pk::pq_t q; q.push_back(a); return q[0] == "e"; endfunction
  function int c1(string a, string b); cq_t q; q.push_back(a); return q[0] == b; endfunction
  function int c2(string a); cq_t q; q.push_back(a); return q[0] >= "e"; endfunction
  function int l1(string a, string b); gq2_t q; q.push_back(a); return q[0] == b; endfunction
  task t1(string a, string b, output int r); gq_t q; q.push_back(a); r = (q[0] == b); endtask
  function int g1(string a); gq_t q; q.push_back(a); if (q[0] == "e") return 7; return 3; endfunction
  function int g2(string a, string b); gq_t q; q.push_back(a); return (q[0] == b) ? 5 : 6; endfunction
endclass
module top;
  function automatic int mf(string a, string b); gq_t q; q.push_back(a); return q[0] == b; endfunction
  initial begin
    C c; int r, r2; c = new;
    repeat (2) begin
      $display("T|a1 %0d %0d %0d %0d", c.a1("e","e"), c.a1("ab","ab"), c.a1("xe","e"), c.a1("[0]","e"));
      $display("T|a2 %0d %0d %0d", c.a2("e"), c.a2("ae"), c.a2("ee"));
      $display("T|a3 %0d %0d %0d", c.a3("e","f"), c.a3("ab","ab"), c.a3("b","ab"));
      $display("T|a4 %0d %0d %0d", c.a4("e","e"), c.a4("ab","ab"), c.a4("xe","e"));
      $display("T|a5 %0d %0d %0d", c.a5("e","e"), c.a5("ab","ab"), c.a5("xe","e"));
      $display("T|a7 %0d %0d %0d", c.a7("e","e"), c.a7("ab","ab"), c.a7("xe","e"));
      $display("T|d %0d %0d %0d %0d %0d", c.d1("e","e"), c.d1("ab","ab"), c.d1("xe","e"), c.d2("e"), c.d2("ae"));
      $display("T|s %0d %0d %0d %0d %0d", c.s1("e","e"), c.s1("ab","ab"), c.s1("xe","e"), c.s2("e"), c.s2("ae"));
      $display("T|p %0d %0d %0d %0d %0d", c.p1("e","e"), c.p1("ab","ab"), c.p1("xe","e"), c.p2("e"), c.p2("ae"));
      $display("T|c %0d %0d %0d %0d %0d", c.c1("e","e"), c.c1("ab","ab"), c.c1("xe","e"), c.c2("e"), c.c2("a"));
      $display("T|l %0d %0d %0d", c.l1("e","e"), c.l1("ab","ab"), c.l1("xe","e"));
      c.t1("ab","ab", r); c.t1("xe","e", r2); $display("T|t %0d %0d", r, r2);
      $display("T|g %0d %0d %0d %0d", c.g1("e"), c.g1("xe"), c.g2("ab","ab"), c.g2("xe","e"));
      $display("T|m %0d %0d %0d", mf("e","e"), mf("ab","ab"), mf("xe","e"));
    end
  end
endmodule
"#;

/// Neighbours: the string-collection typedef declared in a package, in the
/// class, at file scope and through an alias of one; dynamic and associative
/// typedefs; a task and a module function local; the compare in `if`, `?:`
/// and against a literal.
#[test]
fn typedef_string_collection_scopes() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let want = [
        "T|a1 1 1 0 0",
        "T|a2 1 0 0",
        "T|a3 1 0 0",
        "T|a4 1 1 0",
        "T|a5 0 0 1",
        "T|a7 1 1 0",
        "T|d 1 1 0 1 0",
        "T|s 1 1 0 1 0",
        "T|p 1 1 0 1 0",
        "T|c 1 1 0 1 0",
        "T|l 1 1 0",
        "T|t 1 0",
        "T|g 7 3 5 6",
        "T|m 1 1 0",
        "T|a1 1 1 0 0",
        "T|a2 1 0 0",
        "T|a3 1 0 0",
        "T|a4 1 1 0",
        "T|a5 0 0 1",
        "T|a7 1 1 0",
        "T|d 1 1 0 1 0",
        "T|s 1 1 0 1 0",
        "T|p 1 1 0 1 0",
        "T|c 1 1 0 1 0",
        "T|l 1 1 0",
        "T|t 1 0",
        "T|g 7 3 5 6",
        "T|m 1 1 0",
    ];
    assert_eq!(t_lines(TYPEDEF_SCOPES), want);
}

const MEMBER_ELEMENTS: &str = r#"
typedef string sq_t[$];
class C;
  string mq[$];
  sq_t tq;
  function int f6(string a, string b); mq.push_back(a); return mq[mq.size()-1] == b; endfunction
  function int f7(string a, string b); mq.push_back(a); return mq[$] == b; endfunction
  function int f9(string b); return mq[0] == b; endfunction
  function int t1(string a, string b); tq.push_back(a); return tq[tq.size()-1] == b; endfunction
  function int t2(string b); return tq[0] < b; endfunction
endclass
module top;
  initial begin
    C c; c = new;
    $display("T|f6 %0d %0d %0d", c.f6("e", "e"), c.f6("ab", "ab"), c.f6("xe", "e"));
    $display("T|f7 %0d %0d", c.f7("e", "e"), c.f7("xe", "e"));
    $display("T|f9 %0d %0d", c.f9("e"), c.f9("x"));
    $display("T|t1 %0d %0d %0d", c.t1("e", "e"), c.t1("ab", "ab"), c.t1("xe", "e"));
    $display("T|t2 %0d %0d", c.t2("f"), c.t2("a"));
  end
endmodule
"#;

/// An element of an instance-member string queue — declared `string` or
/// through a typedef — compares as a string in a compiled body: it took the
/// comparison context's one-bit width, so `mq[i] == b` was always 0.
#[test]
fn member_string_queue_element_compares_as_string() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    assert_eq!(
        t_lines(MEMBER_ELEMENTS),
        [
            "T|f6 1 1 0",
            "T|f7 1 0",
            "T|f9 1 0",
            "T|t1 1 1 0",
            "T|t2 1 0"
        ]
    );
}
