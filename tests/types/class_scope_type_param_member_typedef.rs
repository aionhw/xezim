//! §8.25.1, §6.20.3: a typedef of a parameterized class that names a member
//! type of its type parameter (`class extractor #(type T); typedef T::hw_t
//! hw_t; endclass`) is that member type in the specialization a class scope
//! names, so `extractor#(cfg_t)::hw_t` is `cfg_t::hw_t`; and a type parameter
//! or typedef bound to a class names that class as a scope (`C::hw_t`).
//! These came out as 32-bit types, with no diagnostic, and
//! writes to the struct's members were lost.
//!
//! Expected values are Verilator's (5.046) for the first test, and for all
//! but `d1` in the last. Verilator rejects the type-parameter default of
//! the second (`Expecting a data type`); in the third it prints 128 for
//! both instances, where §6.20.3 binds `C` to `cfg2_t` in `w2`, whose
//! `hw_t` is 16 bits; and in the last it gives `d1` 128, where the default
//! `U = T` takes the argument `cfg2_t` (§6.20.2).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn assert_has(got: &[String], want: &str) {
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

const CLASSES: &str = r#"
package p;
  typedef struct packed { logic [99:0] a; logic [27:0] b; } s_t;
endpackage
class cfg_t;
  typedef p::s_t hw_t;
endclass
class cfg2_t;
  typedef logic [15:0] hw_t;
endclass
class extractor #(type T = int);
  typedef T::hw_t hw_t;
endclass
"#;

/// A variable, a typedef and a typedef of the specialization, positional
/// and named arguments.
#[test]
fn specialization_names_the_argument_member_type() {
    let got = lines(&format!(
        "{CLASSES}{}",
        r#"
module top;
  extractor#(cfg_t)::hw_t v1;
  typedef extractor#(cfg_t)::hw_t h_t;
  h_t v2;
  typedef extractor#(cfg_t) e_t;
  e_t::hw_t v3;
  extractor#(.T(cfg_t))::hw_t v4;
  extractor#(cfg2_t)::hw_t v5;
  initial begin
    v1 = '0;
    v1.a = 100'h5;
    $display("s1: v1=%0d v2=%0d v3=%0d v4=%0d v5=%0d a=%h", $bits(v1), $bits(v2),
             $bits(v3), $bits(v4), $bits(v5), v1.a);
  end
endmodule
"#
    ));
    assert_has(
        &got,
        "s1: v1=128 v2=128 v3=128 v4=128 v5=16 a=0000000000000000000000005",
    );
}

/// A type parameter's default names a typedef of a specialization by an
/// earlier type parameter, as the CHIPS Alliance I3C core's
/// `axi_adapter_wrapper.sv` does; the variable drives a 128-bit port.
#[test]
fn type_parameter_default_through_a_specialization() {
    let got = lines(&format!(
        "{CLASSES}{}",
        r#"
module sub (input p::s_t x);
  initial #1 $display("s2: sub $bits(x)=%0d x.a=%h", $bits(x), x.a);
endmodule
module wrap #(parameter type C = cfg_t, parameter type H = extractor#(C)::hw_t) ();
  H h;
  initial begin
    h = '0;
    h.a = 100'h123;
    $display("s2: wrap $bits(H)=%0d", $bits(H));
  end
  sub u (.x(h));
endmodule
module top;
  wrap w ();
endmodule
"#
    ));
    assert_has(&got, "s2: wrap $bits(H)=128");
    assert_has(&got, "s2: sub $bits(x)=128 x.a=0000000000000000000000123");
}

/// A type parameter used as a class scope, bound per instance. §6.18
/// allows such a prefix only in a typedef declaration (`C::hw_t h;` is
/// illegal), so the scope is named by module and block typedefs.
#[test]
fn type_parameter_as_a_class_scope() {
    let got = lines(&format!(
        "{CLASSES}{}",
        r#"
module wrap #(parameter type C = cfg_t) ();
  typedef C::hw_t h_t;
  h_t h;
  initial begin
    typedef C::hw_t l_t;
    l_t l;
    #1 $display("s3: %m $bits=%0d local=%0d", $bits(h), $bits(l));
  end
endmodule
module top;
  wrap w ();
  wrap #(.C(cfg2_t)) w2 ();
endmodule
"#
    ));
    assert_has(&got, "s3: top.w $bits=128 local=128");
    assert_has(&got, "s3: top.w2 $bits=16 local=16");
}

/// A type parameter whose default names an earlier one, a typedef of a
/// specialization used as a scope, and an argument that is itself a
/// specialization of a class with a value parameter.
#[test]
fn dependent_defaults_aliases_and_nested_specializations() {
    let got = lines(&format!(
        "{CLASSES}{}",
        r#"
class cfgw #(int W = 4);
  typedef logic [W-1:0] hw_t;
endclass
class pair #(type T = cfg_t, type U = T);
  typedef U::hw_t hw_t;
endclass
module top;
  pair#(cfg2_t)::hw_t d1;
  pair#(cfg2_t, cfg_t)::hw_t d2;
  typedef extractor#(cfg2_t) e2_t;
  e2_t::hw_t a1;
  typedef cfgw#(8) cfg8_t;
  extractor#(cfg8_t)::hw_t n1;
  cfg8_t::hw_t n2;
  initial $display("s4: d1=%0d d2=%0d a1=%0d n1=%0d n2=%0d", $bits(d1), $bits(d2), $bits(a1),
                   $bits(n1), $bits(n2));
endmodule
"#
    ));
    assert_has(&got, "s4: d1=16 d2=128 a1=16 n1=8 n2=8");
}

/// §8.25.1: the argument is itself a specialization of a class with a
/// value parameter, written in place (`extractor#(cfgw#(8))::hw_t`).
/// Expected value is the reference simulator's.
#[test]
fn specialization_argument_written_in_place() {
    let got = lines(&format!(
        "{CLASSES}{}",
        r#"
class cfgw #(int W = 4);
  typedef logic [W-1:0] hw_t;
endclass
module top;
  extractor#(cfgw#(8))::hw_t n1;
  extractor#(cfgw#(.W(6)))::hw_t n2;
  initial $display("s6: n1=%0d n2=%0d", $bits(n1), $bits(n2));
endmodule
"#
    ));
    assert_has(&got, "s6: n1=8 n2=6");
}

/// A typedef of a value-parameterized specialization used as a scope
/// names that class's typedef, not a same-named typedef outside it: the
/// member's signedness comes from the class (§8.25.1).
#[test]
fn alias_scope_keeps_the_class_typedef() {
    let got = lines(
        r#"
typedef logic [7:0] m;
class C #(int N = 1);
  typedef logic signed [7:0] m;
endclass
module top;
  typedef C#(2) c_t;
  c_t::m v;
  C#(2)::m w;
  initial begin
    v = 8'hff;
    w = 8'hff;
    $display("g1: v<0=%0d w<0=%0d bits=%0d", v < 0, w < 0, $bits(v));
  end
endmodule
"#,
    );
    assert_has(&got, "g1: v<0=1 w<0=1 bits=8");
}
