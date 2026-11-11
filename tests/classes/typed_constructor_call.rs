//! IEEE 1800-2023 §8.8: a typed constructor call `C::new` or `C::new(args)`
//! builds a `C`, whatever class the handle it is assigned to is declared
//! with. Expected values are the reference simulator's.

use xezim::simulate;

const CLASSES: &str = r#"
class Base;
  int id;
  function new(int v = 1); id = v; endfunction
  virtual function int kind(); return 1; endfunction
endclass
class Derived extends Base;
  function new(int v = 2); super.new(v); endfunction
  virtual function int kind(); return 2; endfunction
endclass
class P #(int N = 1) extends Base;
  function new(); super.new(N); endfunction
  virtual function int kind(); return 10 + N; endfunction
endclass
typedef Derived dalias;
class Maker;
  Base mem;
  function Base make(); Base t; t = Derived::new(7); return t; endfunction
  function Base make_bare(); Base t; t = Derived::new; return t; endfunction
  function Base make_init(); Base t = Derived::new(8); return t; endfunction
  function void fill(); mem = Derived::new(11); endfunction
endclass
"#;

const SHOW: &str = r#"
  function automatic string show(Base h);
    if (h == null) return "null";
    return $sformatf("%0d/%0d", h.kind(), h.id);
  endfunction
"#;

fn run(module_body: &str) -> Vec<String> {
    let _ = super::compiled_method_test_env::eager();
    let src = format!("{CLASSES}\nmodule top;\n{SHOW}\n{module_body}\nendmodule\n");
    let sim = simulate(&src, 10).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(o: &[String], want: &str) {
    assert!(o.iter().any(|l| l == want), "missing {want:?} in {o:?}");
}

#[test]
fn a_typed_constructor_into_a_base_handle_builds_the_named_class() {
    let o = run(r#"
  Base a, c, e, arr[2];
  Base dinit = Derived::new();
  initial begin
    a = Derived::new();
    c = Derived::new(5);
    e = dalias::new();
    arr[1] = Derived::new(12);
    $display("T|a=%s c=%s e=%s arr1=%s dinit=%s", show(a), show(c), show(e), show(arr[1]), show(dinit));
  end
"#);
    expect(&o, "T|a=2/2 c=2/5 e=2/2 arr1=2/12 dinit=2/2");
}

#[test]
fn a_typed_constructor_without_parentheses_constructs() {
    let o = run(r#"
  function automatic Base fbare(); Base t; t = Derived::new; return t; endfunction
  Base b, g, m;
  Maker mk;
  initial begin
    b = Derived::new;
    g = fbare();
    mk = new;
    m = mk.make_bare();
    $display("T|b=%s fbare=%s make_bare=%s", show(b), show(g), show(m));
  end
"#);
    expect(&o, "T|b=2/2 fbare=2/2 make_bare=2/2");
}

#[test]
fn typed_constructors_that_already_built_the_right_class_keep_working() {
    let o = run(r#"
  function automatic Base fmake(); Base t; t = Derived::new(); return t; endfunction
  function automatic Base finit(); Base t = Derived::new(4); return t; endfunction
  Base d, n, g, h, k, k3;
  Derived x;
  Maker mk;
  initial begin
    x = Derived::new();
    d = P#(3)::new();
    n = Base::new();
    g = fmake();
    h = finit();
    mk = new;
    k = mk.make();
    k3 = mk.make_init();
    mk.fill();
    $display("T|x=%s p=%s base=%s fmake=%s finit=%s", show(x), show(d), show(n), show(g), show(h));
    $display("T|make=%s make_init=%s fill=%s", show(k), show(k3), show(mk.mem));
  end
"#);
    expect(&o, "T|x=2/2 p=13/3 base=1/1 fmake=2/2 finit=2/4");
    expect(&o, "T|make=2/7 make_init=2/8 fill=2/11");
}

#[test]
fn typed_constructors_through_declarations_type_parameters_typedefs_and_packages() {
    let _ = super::compiled_method_test_env::eager();
    let src = format!(
        "{}{CLASSES}{}",
        r#"
package pk;
  class PB; int id = 1; virtual function int kind(); return 1; endfunction endclass
  class PD extends PB; function new(); id = 2; endfunction virtual function int kind(); return 2; endfunction endclass
endpackage
"#,
        r#"
class Q #(int N = 1) extends Base;
  function new(); super.new(N); endfunction
  virtual function int kind(); return 20 + N; endfunction
endclass
typedef Q#(4) q4_t;
class G #(type T = Derived);
  function Base mk(); Base t; t = T::new(); return t; endfunction
  function Base mk_init(); Base t = Derived::new; return t; endfunction
endclass
module top;
  function automatic string show(Base h);
    if (h == null) return "null";
    return $sformatf("%0d/%0d", h.kind(), h.id);
  endfunction
  function automatic Base fdecl(); Base t = Derived::new; return t; endfunction
  Base a, c, d, f, f2;
  pk::PB pa, pb;
  G g;
  initial begin
    a = fdecl();
    g = new;
    c = g.mk();
    d = g.mk_init();
    f = q4_t::new();
    f2 = q4_t::new;
    pa = pk::PD::new();
    pb = pk::PD::new;
    $display("T|fdecl=%s tparam=%s mdecl=%s tdspec=%s tdspecbare=%s", show(a), show(c), show(d), show(f), show(f2));
    $display("T|pkg=%0d/%0d pkgbare=%0d/%0d", pa.kind(), pa.id, pb.kind(), pb.id);
  end
endmodule
"#
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    let o: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    expect(
        &o,
        "T|fdecl=2/2 tparam=2/2 mdecl=2/2 tdspec=24/4 tdspecbare=24/4",
    );
    expect(&o, "T|pkg=2/2 pkgbare=2/2");
}
