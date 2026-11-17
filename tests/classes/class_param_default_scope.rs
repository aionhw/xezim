//! IEEE 1800-2017 §6.20.2 / §8.25: a class parameter's DEFAULT is evaluated
//! in the class's own scope, where the parameters declared before it are
//! visible with the values of THE specialization being built. And §8.25:
//! two spellings of one specialization (`C#(int,3)` and `C#(int,3,6)` when
//! the third parameter defaults to 6) are the same class, with one set of
//! statics.
//!
//! Before the fix a default was evaluated in the scope of the `new` call:
//! `int D = W*2` read `W` as x, or as a same-named parameter of the calling
//! module, and the specialization signature kept the unfolded default, so
//! the two spellings of one specialization kept separate statics. A type
//! default that names an earlier type parameter (`type U = T`) and a
//! named parameter assignment (`#(.W(5))`, whose name the parser dropped)
//! were wrong the same way. Expected values are the reference simulator's.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect_all(o: &[String], want: &[&str]) {
    for line in want {
        assert!(o.iter().any(|l| l == line), "missing `{line}`: {o:?}");
    }
}

/// `D = W*2` follows `W` in every way a specialization is reached: directly,
/// through a typedef, with every parameter defaulted, through a
/// non-parameterized and a parameterized subclass, and through a subclass
/// that forwards its own parameter. The packed width `bit [D-1:0]` and the
/// static method follow too, and a module parameter named `W` stays out.
#[test]
fn dependent_value_default_follows_each_specialization() {
    let o = lines(
        r#"
class pbase #(type T = int, int W = 8, int D = W*2);
  bit [D-1:0] v;
  static int cnt;
  function void show(string who);
    $display("T|%s %0d %0d %0d %0d", who, $bits(T), W, D, $bits(v));
  endfunction
  task show_t(string who);
    $display("T|%s t %0d %0d %0d", who, $bits(T), W, D);
  endtask
  static function int sd(); return D; endfunction
endclass
class dimp extends pbase; endclass
class dpar #(int X = 1) extends pbase; endclass
class dfwd #(int X = 3) extends pbase #(int, X); endclass
typedef pbase #(int, 2) p2_t;
module m #(parameter int W = 5, parameter int D = W*2) ();
  initial $display("T|mod %0d %0d", W, D);
endmodule
module tb;
  m u0();
  m #(7) u1();
  initial begin
    pbase #(int, 3) a = new;
    p2_t b = new;
    pbase c = new;
    dimp d = new;
    dpar e = new;
    dfwd f = new;
    dfwd #(6) g = new;
    pbase #(byte, 4) h = new;
    #1;
    a.show("a"); b.show("b"); c.show("c"); d.show("d"); e.show("e");
    f.show("f"); g.show("g"); h.show("h");
    d.show_t("d"); e.show_t("e"); f.show_t("f");
    $display("T|sd %0d %0d %0d %0d", pbase#(int, 3)::sd(), p2_t::sd(), pbase#()::sd(), pbase#(byte, 4)::sd());
    begin pbase#(int, 3)::cnt += 5; end
    begin pbase#()::cnt += 9; end
    $display("T|share %0d %0d %0d", pbase#(int, 3, 6)::cnt, pbase#(int, 3, 7)::cnt, pbase#(int, 8, 16)::cnt);
    $display("T|inherit %0d %0d", dimp::cnt, dimp::sd());
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &[
            "T|mod 5 10",
            "T|mod 7 14",
            "T|a 32 3 6 6",
            "T|b 32 2 4 4",
            "T|c 32 8 16 16",
            "T|d 32 8 16 16",
            "T|e 32 8 16 16",
            "T|f 32 3 6 6",
            "T|g 32 6 12 12",
            "T|h 8 4 8 8",
            "T|d t 32 8 16",
            "T|e t 32 8 16",
            "T|f t 32 3 6",
            "T|sd 6 4 16 8",
            "T|share 5 0 9",
            "T|inherit 9 16",
        ],
    );
}

/// A type default naming an earlier type parameter, a value default sized
/// from a type parameter, and a default reading a package parameter; a
/// class-body localparam then follows the header default.
#[test]
fn type_and_package_defaults_follow_the_specialization() {
    let o = lines(
        r#"
package pk;
  parameter int PP = 7;
endpackage
class tdep #(type T = int, type U = T, int W = $bits(U));
  function void show(string who); $display("T|%s %0d %0d %0d", who, $bits(T), $bits(U), W); endfunction
endclass
class lpd #(int A = 3, int B = A * pk::PP);
  localparam int L = B + 1;
  function void show(string who); $display("T|%s %0d %0d %0d", who, A, B, L); endfunction
endclass
module tb;
  initial begin
    tdep f = new;
    tdep #(byte) g = new;
    tdep #(byte, shortint) h = new;
    lpd k = new;
    lpd #(2) l = new;
    f.show("f"); g.show("g"); h.show("h");
    k.show("k"); l.show("l");
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &[
            "T|f 32 32 32",
            "T|g 8 8 8",
            "T|h 8 16 16",
            "T|k 3 21 22",
            "T|l 2 14 15",
        ],
    );
}

/// Other default shapes in the class scope: a package parameter named bare
/// inside its package, `$clog2`, string concatenation, a negated and a real
/// default, a class declared in a module, a static method; and a class-body
/// localparam of a NON-parameterized subclass that reads an inherited
/// parameter.
#[test]
fn other_default_shapes_follow_the_specialization() {
    let o = lines(
        r#"
package p2;
  parameter int Q = 5;
  class pc #(int A = 2, int B = A + Q);
    function void show(string who); $display("T|%s %0d %0d", who, A, B); endfunction
  endclass
endpackage
class base #(type T = int, int W = 8, int D = W*2);
endclass
class d2 extends base #(int, 3);
  localparam int L = D + 1;
  function void show(); $display("T|d2 %0d %0d", D, L); endfunction
endclass
class misc #(int DEPTH = 16, int AW = $clog2(DEPTH), string S = "ab", string S2 = {S, "cd"}, int N = -DEPTH, real R = DEPTH * 1.5);
  function void show(string who); $display("T|%s %0d %0d %s %s %0d %0.1f", who, DEPTH, AW, S, S2, N, R); endfunction
  static function int saw(); return AW; endfunction
endclass
module tb;
  class inmod #(int A = 1, int B = A*3);
    function void show(string who); $display("T|%s %0d %0d", who, A, B); endfunction
  endclass
  initial begin
    p2::pc a = new;
    p2::pc #(10) b = new;
    d2 c = new;
    misc m1 = new;
    misc #(64) m2 = new;
    misc #(8, 5, "zz") m3 = new;
    inmod i1 = new;
    inmod #(4) i2 = new;
    a.show("a"); b.show("b"); c.show();
    m1.show("m1"); m2.show("m2"); m3.show("m3");
    i1.show("i1"); i2.show("i2");
    $display("T|saw %0d %0d", misc#(64)::saw(), misc#()::saw());
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &[
            "T|a 2 7",
            "T|b 10 15",
            "T|d2 6 7",
            "T|m1 16 4 ab abcd -16 24.0",
            "T|m2 64 6 ab abcd -64 96.0",
            "T|m3 8 5 zz zzcd -8 12.0",
            "T|i1 1 3",
            "T|i2 4 12",
            "T|saw 6 4",
        ],
    );
}

/// §8.25 named parameter assignment binds by NAME — in a declaration, in
/// any order, through a typedef, in an `extends` clause and in a static
/// call — and the unnamed parameters keep their (dependent) defaults.
#[test]
fn named_parameter_assignment_binds_by_name() {
    let o = lines(
        r#"
class nb #(type T = int, int W = 8, int D = W*2, type U = T);
  bit [D-1:0] v;
  function void show(string who); $display("T|%s %0d %0d %0d %0d %0d", who, $bits(T), W, D, $bits(U), $bits(v)); endfunction
  static function int sd(); return D; endfunction
endclass
class dn #(int X = 3) extends nb #(.W(X)); endclass
class dn2 extends nb #(.U(byte), .D(5)); endclass
typedef nb #(.W(6)) n6_t;
module tb;
  initial begin
    nb #(.W(4)) n1 = new;
    nb #(.D(5)) n2 = new;
    nb #(.U(byte)) n3 = new;
    nb #(.T(shortint), .D(9)) n4 = new;
    nb #(.D(3), .W(1)) n5 = new;
    n6_t n6 = new;
    dn z = new;
    dn2 y = new;
    n1.show("n1"); n2.show("n2"); n3.show("n3"); n4.show("n4"); n5.show("n5"); n6.show("n6");
    z.show("z"); y.show("y");
    $display("T|sd %0d %0d %0d", nb#(.W(5))::sd(), nb#(.D(11))::sd(), n6_t::sd());
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &[
            "T|n1 32 4 8 32 8",
            "T|n2 32 8 5 32 5",
            "T|n3 32 8 16 8 16",
            "T|n4 16 8 9 16 9",
            "T|n5 32 1 3 32 3",
            "T|n6 32 6 12 32 12",
            "T|z 32 3 6 32 6",
            "T|y 32 8 5 8 5",
            "T|sd 10 11 12",
        ],
    );
}

/// §26.3: a method of a class DECLARED inside a package reads a bare
/// package parameter/variable of that package even when the running module
/// neither imports it nor declares it — and that package scope beats a
/// same-named module parameter (class bodies scope through their declaring
/// package before the module instance scope), as does a package function
/// body that shadows the module scope. A class property of the same
/// name still beats both.
#[test]
fn package_class_method_reads_own_package_bare() {
    let o = lines(
        r#"
package p2;
  parameter int Q = 5;
  function int bump(int x); return x + Q; endfunction
  class pc;
    function void show(string tag);
      $display("T|%s %0d", tag, Q);
    endfunction
    function void callpkg(string tag); $display("T|%s %0d", tag, bump(10)); endfunction
  endclass
  class pc2;
    int Q = 7;
    function void show(string tag); $display("T|%s %0d", tag, Q); endfunction
  endclass
endpackage
module tb;
  parameter int Q = 99;
  initial begin
    p2::pc a = new;
    p2::pc2 b = new;
    a.show("bare");
    b.show("prop");
    a.callpkg("bump");
    $display("T|mod %0d", Q);
    $display("T|pfn %0d", p2::bump(99));
    $finish;
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &["T|bare 5", "T|prop 7", "T|bump 15", "T|mod 99", "T|pfn 104"],
    );
}
/// Module-scope class variables — in the top module and in an instantiated
/// module, whose `#(int, P)` folds that instance's `P` — take their
/// specialization, named or positional, and `$typename` (§20.6.1) lists
/// every parameter, a named one at its own slot and a default that names an
/// earlier parameter at that parameter's value.
#[test]
fn module_scope_declarations_and_typename() {
    let o = lines(
        r#"
class nb #(type T = int, int W = 8, int D = W*2, type U = T);
  function void show(string who); $display("T|%s %0d %0d %0d %0d", who, $bits(T), W, D, $bits(U)); endfunction
endclass
class sb #(string S = "ab", int K = 2);
endclass
module sub #(parameter int P = 3) ();
  nb #(int, P) o;
  nb #(.W(P)) o2;
  initial begin #1; o = new; o.show("subpos"); o2 = new; o2.show("subnamed"); end
endmodule
module tb;
  nb #(int, 7) g4 = new;
  nb #(.W(5)) g2 = new;
  nb #(.T(byte), .D(3)) g5 = new;
  nb g6 = new;
  nb #(shortint) g7 = new;
  sb s1 = new;
  sub #(9) s9();
  sub s3();
  initial begin
    g4.show("g4"); g2.show("g2"); g5.show("g5");
    $display("T|%s", $typename(g2));
    $display("T|%s", $typename(g4));
    $display("T|%s", $typename(g5));
    $display("T|%s", $typename(g6));
    $display("T|%s", $typename(g7));
    $display("T|%s", $typename(s1));
  end
endmodule
"#,
    );
    expect_all(
        &o,
        &[
            "T|g4 32 7 14 32",
            "T|g2 32 5 10 32",
            "T|g5 8 8 3 8",
            "T|class nb #(int, 5, 10, int)",
            "T|class nb #(int, 7, 14, int)",
            "T|class nb #(byte, 8, 3, byte)",
            "T|class nb #(int, 8, 16, int)",
            "T|class nb #(shortint, 8, 16, shortint)",
            "T|class sb #(\"ab\", 2)",
            "T|subpos 32 9 18 32",
            "T|subnamed 32 9 18 32",
            "T|subpos 32 3 6 32",
            "T|subnamed 32 3 6 32",
        ],
    );
}
