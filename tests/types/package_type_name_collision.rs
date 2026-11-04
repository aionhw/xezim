//! §26.3 — a type name declared in more than one package (issue #300,
//! reported by Ganesh-TNL).
//!
//! Typedef widths, typedef targets, struct layouts, enum members and classes
//! live in flat tables keyed by name. Two packages declaring `blk_t` (or a
//! package and `$unit`, or a package and a module) shared one entry, so the
//! package registered last won everywhere: a class in the first package got
//! the other package's struct for its `rand` member, `$bits(pkg_one::blk_t)`
//! read the other width, and `randomize()` failed. Same-named classes
//! collapsed the same way (`pa::C` ran `pb::C`'s methods), and a bare enum
//! member imported from one package read another package's value.
//!
//! The fix renames a package type declared somewhere else too to its
//! qualified spelling before elaboration and resolves every reference by
//! the §26.3 rules: inside package P, P's declaration wins; then local
//! declarations, explicit imports and wildcard imports, scope by scope; two
//! wildcard imports providing different declarations make the name
//! invisible (an error where it is used as a type).
//!
//! Expected values come from the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn t_lines(src: &str) -> Vec<String> {
    lines(src)
        .into_iter()
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn assert_t_lines(src: &str, expected: &[&str]) {
    let got = t_lines(src);
    assert_eq!(got, expected, "T| lines differ");
}

/// The issue's reproducer: a class in `pkg_one` randomizes a member of
/// `pkg_one::blk_t` while `pkg_two` declares another `blk_t`.
#[test]
fn issue_300_same_typedef_name_in_two_packages() {
    let out = lines(include_str!("issue_300_typedef_collision.sv"));
    assert!(
        out.iter()
            .any(|m| m == "BITS pkg_one::blk_t=976  pkg_two::blk_t=12288"),
        "{out:?}"
    );
    assert_eq!(
        out.iter()
            .filter(|m| *m == "randomize OK   : n_units=1 first_unit=0")
            .count(),
        3,
        "{out:?}"
    );
    assert!(out.iter().any(|m| m == "TEST_PASS"), "{out:?}");
}

/// §23.9/§26.3: a nested local typedef shadows only its own block.
#[test]
fn child_block_typedef_preserves_enclosing_package_import() {
    assert_t_lines(
        r#"
package copper; typedef bit [3:0] word_t; endpackage
package silver; typedef bit [7:0] word_t; endpackage
module scope_probe;
  import copper::*;
  function void report();
    word_t outer_word;
    $display("T|function outer=%0d", $bits(outer_word));
    begin
      typedef bit [1:0] word_t;
      word_t inner_word;
      $display("T|function inner=%0d", $bits(inner_word));
    end
    begin
      word_t sibling_word;
      $display("T|function sibling=%0d", $bits(sibling_word));
    end
  endfunction
  initial begin
    word_t outer_word;
    $display("T|initial outer=%0d", $bits(outer_word));
    begin
      typedef bit [1:0] word_t;
      word_t inner_word;
      $display("T|initial inner=%0d", $bits(inner_word));
    end
    begin
      word_t sibling_word;
      $display("T|initial sibling=%0d", $bits(sibling_word));
    end
    report();
  end
endmodule
"#,
        &[
            "T|initial outer=4",
            "T|initial inner=2",
            "T|initial sibling=4",
            "T|function outer=4",
            "T|function inner=2",
            "T|function sibling=4",
        ],
    );
}

/// §27/§26.3: a generate-local typedef does not shadow its parent or sibling.
#[test]
fn generate_block_typedef_preserves_enclosing_package_import() {
    assert_t_lines(
        r#"
package copper; typedef bit [3:0] word_t; endpackage
package silver; typedef bit [7:0] word_t; endpackage
module scope_probe;
  import copper::*;
  word_t outer_word;
  if (1) begin : local_bank
    typedef bit [1:0] word_t;
    word_t inner_word;
    initial $display("T|generate inner=%0d", $bits(inner_word));
  end
  if (1) begin : imported_bank
    word_t sibling_word;
    initial $display("T|generate sibling=%0d", $bits(sibling_word));
  end
  for (genvar index = 0; index < $bits(word_t); index++) begin : loop_bank
    typedef bit [1:0] word_t;
    word_t loop_word;
    initial $display("T|generate loop=%0d", $bits(loop_word));
  end
  case (1) 1: begin : chosen_bank
    word_t case_word;
    initial $display("T|generate case=%0d", $bits(case_word));
  end endcase
  initial $display("T|generate outer=%0d", $bits(outer_word));
endmodule
"#,
        &[
            "T|generate inner=2",
            "T|generate sibling=4",
            "T|generate loop=2",
            "T|generate loop=2",
            "T|generate loop=2",
            "T|generate loop=2",
            "T|generate case=4",
            "T|generate outer=4",
        ],
    );
}

/// Struct, enum, union and plain typedefs of the same name in two packages:
/// `$bits` through the qualified name and inside each package, class
/// members, constraints on struct fields, casts, assignment patterns and
/// enum `.name()`.
const TYPES: &str = r#"
package pa;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  typedef enum logic [1:0] { RED, GRN } e_t;
  typedef union packed { logic [5:0] x; logic [5:0] y; } u_t;
  typedef logic [6:0] w_t;
  typedef s_t sarr_t [2];
  function automatic int fbits(); return $bits(s_t) + 100*$bits(e_t) + 10000*$bits(u_t); endfunction
  function automatic int wbits(); return $bits(w_t); endfunction
  class C;
    rand s_t s; rand u_t u; e_t e; w_t w;
    constraint c1 { s.a == 4'd3; s.b == 4'd9; }
    function int sz(); return $bits(s) + 100*$bits(u) + 10000*$bits(e) + 1000000*$bits(w); endfunction
  endclass
endpackage
package pb;
  typedef struct packed { logic [15:0] a; logic [7:0] b; logic [7:0] c; } s_t;
  typedef enum logic [4:0] { BLUE, CYAN, MAG } e_t;
  typedef union packed { logic [20:0] x; logic [20:0] y; } u_t;
  typedef logic [12:0] w_t;
  function automatic int fbits(); return $bits(s_t) + 100*$bits(e_t) + 10000*$bits(u_t); endfunction
  function automatic int wbits(); return $bits(w_t); endfunction
  class D;
    rand s_t s; rand u_t u; e_t e; w_t w;
    constraint c1 { s.a == 16'd1234; s.c == 8'd77; }
    function int sz(); return $bits(s) + 100*$bits(u) + 10000*$bits(e) + 1000000*$bits(w); endfunction
  endclass
endpackage
module t1;
  import pa::*;
  import pb::*;
  pa::s_t va; pb::s_t vb;
  pa::w_t wa; pb::w_t wb;
  pa::sarr_t arr;
  initial begin
    automatic pa::C c = new; automatic pb::D d = new;
    $display("T|bits s %0d %0d", $bits(pa::s_t), $bits(pb::s_t));
    $display("T|bits e %0d %0d", $bits(pa::e_t), $bits(pb::e_t));
    $display("T|bits u %0d %0d", $bits(pa::u_t), $bits(pb::u_t));
    $display("T|bits w %0d %0d", $bits(pa::w_t), $bits(pb::w_t));
    $display("T|bits vars %0d %0d %0d %0d %0d", $bits(va), $bits(vb), $bits(wa), $bits(wb), $bits(arr));
    $display("T|fbits %0d %0d", pa::fbits(), pb::fbits());
    $display("T|wbits %0d %0d", pa::wbits(), pb::wbits());
    $display("T|csz %0d %0d", c.sz(), d.sz());
    $display("T|rand %0d %0d", c.randomize(), d.randomize());
    $display("T|c.s.a=%0d c.s.b=%0d d.s.a=%0d d.s.c=%0d", c.s.a, c.s.b, d.s.a, d.s.c);
    wa = pa::w_t'(16'hffff); wb = pb::w_t'(16'hffff);
    $display("T|cast %h %h", wa, wb);
    $display("T|cast2 %0d %0d", $bits(pa::w_t'(1)), $bits(pb::w_t'(1)));
    va = '{a: 4'hf, b: 4'h1}; vb = '{a: 16'h1234, b: 8'h56, c: 8'h78};
    $display("T|va %p vb %p", va, vb);
    $display("T|enum %0d %0d", pa::GRN, pb::MAG);
    c.e = pa::GRN; d.e = pb::MAG;
    $display("T|enumname %s %s", c.e.name(), d.e.name());
  end
endmodule
"#;

#[test]
fn same_named_typedefs_of_every_kind() {
    assert_t_lines(
        TYPES,
        &[
            "T|bits s 8 32",
            "T|bits e 2 5",
            "T|bits u 6 21",
            "T|bits w 7 13",
            "T|bits vars 8 32 7 13 16",
            "T|fbits 60208 210532",
            "T|wbits 7 13",
            "T|csz 7020608 13052132",
            "T|rand 1 1",
            "T|c.s.a=3 c.s.b=9 d.s.a=1234 d.s.c=77",
            "T|cast 7f 1fff",
            "T|cast2 7 13",
            "T|va '{a:'hf, b:'h1} vb '{a:'h1234, b:'h56, c:'h78}",
            "T|enum 1 2",
            "T|enumname GRN MAG",
        ],
    );
}

/// Same-named classes, base classes, out-of-block methods, parameterized
/// classes, an explicit import beating a wildcard one (in a package and in
/// a module), `extends pkg::C`, `%m` and `$typename`.
const CLASSES: &str = r#"
package pa;
  typedef logic [3:0] n_t;
  typedef struct packed { n_t a; n_t b; } pair_t;
  typedef class C;
  class B; int bv = 7; function int who(); return 1; endfunction endclass
  class C extends B;
    n_t x;
    extern function int width();
    static function int sw(); return $bits(n_t); endfunction
    function void hi(); $display("T|pa %m"); endfunction
  endclass
  function int C::width(); return $bits(x); endfunction
  class P #(int W = 2); n_t q; logic [W-1:0] r; function int s(); return $bits(q) + 100*$bits(r); endfunction endclass
  function automatic int use_c(); C c = new; return C::sw() + 10*c.who(); endfunction
endpackage
package pb;
  typedef logic [7:0] n_t;
  typedef struct packed { n_t a; n_t b; n_t c; } pair_t;
  class B; int bv = 8; function int who(); return 2; endfunction endclass
  class C extends B;
    n_t x;
    extern function int width();
    static function int sw(); return $bits(n_t); endfunction
    function void hi(); $display("T|pb %m"); endfunction
  endclass
  function int C::width(); return $bits(x); endfunction
  class P #(int W = 3); n_t q; logic [W-1:0] r; function int s(); return $bits(q) + 100*$bits(r); endfunction endclass
  function automatic int use_c(); C c = new; return C::sw() + 10*c.who(); endfunction
endpackage
package pc;
  import pb::n_t;
  import pa::*;
  typedef struct packed { n_t z; } wrap_t;
  class D extends pa::C; pb::n_t y; function int s(); return $bits(x) + 100*$bits(y); endfunction endclass
endpackage
module t4;
  import pa::*;
  import pb::n_t;
  import pc::*;
  n_t m;
  pair_t pr;
  initial begin
    automatic pa::C ca = new;
    automatic pb::C cb = new;
    automatic D d = new;
    automatic pa::P#(5) p5 = new;
    automatic pb::P pbd = new;
    $display("T|m %0d pr %0d wrap %0d", $bits(m), $bits(pr), $bits(wrap_t));
    $display("T|pair %0d %0d", $bits(pa::pair_t), $bits(pb::pair_t));
    $display("T|width %0d %0d sw %0d %0d", ca.width(), cb.width(), pa::C::sw(), pb::C::sw());
    $display("T|base %0d %0d %0d %0d", ca.bv, cb.bv, ca.who(), cb.who());
    $display("T|d %0d %0d %0d", d.s(), d.bv, d.who());
    $display("T|P %0d %0d", p5.s(), pbd.s());
    $display("T|use_c %0d %0d", pa::use_c(), pb::use_c());
    $display("T|tn %s %s %s", $typename(ca), $typename(cb), $typename(d));
    ca.hi(); cb.hi();
  end
endmodule
"#;

#[test]
fn same_named_classes_in_two_packages() {
    assert_t_lines(
        CLASSES,
        &[
            "T|m 8 pr 8 wrap 8",
            "T|pair 8 24",
            "T|width 4 8 sw 4 8",
            "T|base 7 8 1 2",
            "T|d 804 7 1",
            "T|P 504 308",
            "T|use_c 14 28",
            "T|tn class pa::C class pb::C class pc::D",
            "T|pa pa.C.hi",
            "T|pb pb.C.hi",
        ],
    );
}

/// Enum members of the same name in two packages, a `$unit` typedef and a
/// module-local typedef sharing a package typedef's name: inside each
/// package its own declaration wins, a wildcard import beats `$unit`, and a
/// module's own typedef beats an import.
const MEMBERS_AND_SCOPES: &str = r#"
typedef logic [2:0] u_t;
typedef enum { ALPHA = 7, OMEGA = 9 } ue_t;
function automatic int unit_bits(); return $bits(u_t); endfunction
package pa;
  typedef logic [10:0] u_t;
  typedef enum { IDLE, RUN, ALPHA } st_t;
  st_t pv = RUN;
  function automatic int ubits(); return $bits(u_t); endfunction
  function automatic int idle_v(); return IDLE; endfunction
  function automatic int alpha_v(); return ALPHA; endfunction
  class K; st_t s = RUN; u_t u; function int get(); return s; endfunction function int ub(); return $bits(u); endfunction endclass
endpackage
package pb;
  typedef enum { RUN, IDLE } st_t;
  st_t pv = RUN;
  localparam st_t LP = IDLE;
  function automatic int idle_v(); return IDLE; endfunction
  function automatic int mix(); st_t s; s = IDLE; return s + 10*RUN; endfunction
endpackage
module sub;
  typedef logic [4:0] u_t;
  import pa::*;
  u_t mine;
  initial #1 $display("T|sub %0d %0d", $bits(mine), $bits(pa::u_t));
endmodule
module t5;
  import pa::*;
  u_t a;
  st_t st;
  pa::st_t q = RUN;
  st_t r;
  sub s0();
  initial begin
    automatic K k = new;
    st = RUN;
    r = IDLE;
    $display("T|u %0d %0d %0d", $bits(a), unit_bits(), pa::ubits());
    $display("T|idle %0d %0d %0d %0d", IDLE, RUN, pa::idle_v(), pb::idle_v());
    $display("T|alpha %0d %0d %0d", ALPHA, pa::alpha_v(), OMEGA);
    $display("T|st %0d %s mix %0d", st, st.name(), pb::mix());
    $display("T|k %0d %0d", k.get(), k.ub());
    case (st) IDLE: $display("T|case idle"); RUN: $display("T|case run"); default: $display("T|case other"); endcase
    $display("T|q %0d pva %0d pvb %0d lp %0d r %0d", q, pa::pv, pb::pv, pb::LP, r);
    $display("T|inside %0d %0d", r inside {IDLE}, q == RUN);
  end
endmodule
"#;

#[test]
fn same_named_enum_members_and_unit_and_module_typedefs() {
    let out = lines(MEMBERS_AND_SCOPES);
    assert!(
        !out.iter()
            .any(|m| m.contains("redefined with a different value")),
        "{out:?}"
    );
    let t: Vec<&str> = out
        .iter()
        .filter(|m| m.starts_with("T|"))
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        t,
        [
            "T|u 11 3 11",
            "T|idle 0 1 0 1",
            "T|alpha 2 2 9",
            "T|st 1 RUN mix 1",
            "T|k 1 11",
            "T|case run",
            "T|q 1 pva 1 pvb 0 lp 1 r 0",
            "T|inside 1 1",
            "T|sub 5 11",
        ]
    );
}

/// §26.3: a type name two wildcard-imported packages both declare is not
/// visible; using it is an error (the reference simulator rejects it too).
#[test]
fn ambiguous_wildcard_imported_type_is_an_error() {
    let src = r#"
package pa; typedef logic [3:0] n_t; endpackage
package pb; typedef logic [7:0] n_t; endpackage
module t3;
  import pa::*;
  import pb::*;
  n_t v;
  initial $display("T|amb %0d", $bits(v));
endmodule
"#;
    let err = match simulate(src, 10) {
        Ok(_) => panic!("ambiguous wildcard-imported type was accepted"),
        Err(e) => e,
    };
    assert!(err.contains("n_t"), "{err}");
    assert!(err.contains("§26.3"), "{err}");
}
