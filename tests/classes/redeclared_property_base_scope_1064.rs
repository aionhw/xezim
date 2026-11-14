//! §8.14: a property a subclass redeclares hides the base property only in
//! code written in the subclass. Base-class methods, and references through
//! a handle of the base type, keep the base class's property: a method call
//! on it (`f.add()`), a member write through it (`f.n++`, `this.f.n += 10`),
//! a member chain through a base handle (`b.f.tag`, `b.f.inner.n`,
//! `b.f.n = 50`, `b.f.add()`), `super.f` in the subclass, virtual and
//! non-virtual methods, a parameterized base, and base methods that ran on a
//! plain base object first (#1064). Expected values are the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The reporter's reproducer (#1064).
#[test]
fn base_methods_and_base_handles_keep_the_base_property() {
    assert_eq!(
        lines(
            r#"// A subclass property with the same name as a base-class property hides it
// only for code in the subclass (IEEE 1800-2017 8.14). Code in the base class,
// and access through a handle of the base type, still use the base property.
class box;
    string tag;
    int    n;
    function new(string t); tag = t; endfunction
    function void add(); n++; endfunction
endclass

class base;
    box f;
    function new(); f = new("base"); endfunction
    function void add_via_member(); f.add(); endfunction
    function void add_via_field(); f.n++; endfunction
    function box get_f(); return f; endfunction
endclass

class derived extends base;
    box f;
    function new(); super.new(); f = new("derived"); endfunction
endclass

module top;
    int fails = 0;
    task automatic check(string what, string got, string exp);
        if (got != exp) begin
            fails++;
            $display("FAIL %s: got %s, expected %s", what, got, exp);
        end
    endtask
    initial begin
        derived d = new;
        base    b = d;
        check("base handle b.f", b.f.tag, "base");
        check("subclass handle d.f", d.f.tag, "derived");
        check("base method returns f", d.get_f().tag, "base");
        d.add_via_member();
        d.add_via_field();
        check("base methods f.add() and f.n++ reach base f", $sformatf("%0d", d.get_f().n), "2");
        check("base methods leave subclass f alone", $sformatf("%0d", d.f.n), "0");
        $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
        $finish;
    end
endmodule
"#
        ),
        ["TEST PASSED"]
    );
}

/// §8.14, §8.15: the neighbouring receiver forms.
#[test]
fn redeclared_handle_property_in_every_receiver_form() {
    assert_eq!(
        lines(
            r#"class box;
  string tag; int n; box inner;
  function new(string t); tag = t; endfunction
  function void add(); n++; endfunction
  function int get(); return n; endfunction
endclass
class base;
  box f;
  int k;
  function new(); f = new("base"); f.inner = new("base_in"); k = 1; endfunction
  function void add_m(); f.add(); endfunction
  function void add_fld(); f.n++; endfunction
  function void add_this(); this.f.add(); this.f.n += 10; endfunction
  virtual function void vadd(); f.add(); endfunction
  function void deep(); f.inner.n += 100; f.inner.add(); endfunction
  function string tag_of(); return f.tag; endfunction
  function int k_of(); return k; endfunction
  function void set_k(int x); k = x; endfunction
  function box get_f(); return f; endfunction
  function void assign_f(box b); f = b; endfunction
endclass
class derived extends base;
  box f;
  int k;
  function new(); super.new(); f = new("derived"); k = 2; endfunction
  function void d_add(); f.add(); endfunction
  function void s_add(); super.f.add(); super.f.n += 1000; endfunction
  function string s_tag(); return super.f.tag; endfunction
  virtual function void vadd(); f.n += 5; super.vadd(); endfunction
endclass
class pbase #(type T = int);
  box f;
  T val;
  function new(); f = new("pbase"); endfunction
  function void add_m(); f.add(); endfunction
  function string tag_of(); return f.tag; endfunction
endclass
class pder extends pbase #(int);
  box f;
  function new(); super.new(); f = new("pder"); endfunction
endclass
module t;
  base b0, b; derived d; pbase #(int) pb; pder pd;
  initial begin
    // warm up the base methods on a plain base object first
    b0 = new; b0.add_m(); b0.add_fld(); b0.add_this(); b0.deep(); b0.vadd();
    $display("W %0d %0d", b0.f.n, b0.f.inner.n);
    d = new; b = d;
    $display("A %s %s %s %s %s", b.f.tag, d.f.tag, d.get_f().tag, d.tag_of(), d.s_tag());
    d.add_m(); d.add_fld();
    $display("B %0d %0d", d.get_f().n, d.f.n);
    d.add_this();
    $display("C %0d %0d", d.get_f().n, d.f.n);
    d.d_add();
    $display("D %0d %0d", d.get_f().n, d.f.n);
    d.s_add();
    $display("E %0d %0d", d.get_f().n, d.f.n);
    d.vadd();
    $display("F %0d %0d", d.get_f().n, d.f.n);
    b.vadd();
    $display("G %0d %0d", d.get_f().n, d.f.n);
    d.deep();
    $display("H %0d %0d", d.get_f().inner.n, b.f.inner.n);
    b.f.n = 50; b.f.add();
    $display("I %0d %0d %0d", b.f.n, d.f.n, b.f.get());
    d.f.n = 60; d.f.add();
    $display("J %0d %0d %0d", b.f.n, d.f.n, d.f.get());
    $display("K %0d %0d %0d %0d", b.k, d.k, d.k_of(), b.k_of());
    d.set_k(9);
    $display("L %0d %0d", b.k, d.k);
    begin box nb; nb = new("swapped"); d.assign_f(nb); end
    $display("N %s %s", b.f.tag, d.f.tag);
    pd = new; pb = pd;
    pd.add_m();
    $display("P %s %s %s %0d %0d", pb.f.tag, pd.f.tag, pd.tag_of(), pb.f.n, pd.f.n);
  end
endmodule
"#
        ),
        [
            r#"W 14 101"#,
            r#"A base derived base base base"#,
            r#"B 2 0"#,
            r#"C 13 0"#,
            r#"D 13 1"#,
            r#"E 1014 1"#,
            r#"F 1015 6"#,
            r#"G 1016 11"#,
            r#"H 101 101"#,
            r#"I 51 11 51"#,
            r#"J 51 61 61"#,
            r#"K 1 2 1 1"#,
            r#"L 9 2"#,
            r#"N swapped derived"#,
            r#"P pbase pder pbase 1 0"#,
        ]
    );
}

/// §8.9, §8.14: a base static hidden by a derived instance property, and a
/// base instance property hidden by a derived static, through the hidden
/// declaration's class and through the other class's handle.
#[test]
fn static_and_instance_redeclarations_through_chains() {
    assert_eq!(
        lines(
            r#"class box; int n; function void add(); n++; endfunction endclass
class sbase;
  static box f;
  function void base_add(); f.add(); endfunction
endclass
class iderived extends sbase;
  box f;
  function new(); f = new; endfunction
  function void der_add(); f.add(); f.n += 10; endfunction
endclass
class ibase;
  box g;
  function new(); g = new; endfunction
  function void base_add(); g.add(); g.n += 100; endfunction
endclass
class sderived extends ibase;
  static box g;
  function void der_add(); g.add(); endfunction
endclass
module h3;
  iderived d; sbase b; sderived sd; ibase ib;
  initial begin
    sbase::f = new; d = new; b = d;
    d.base_add(); d.der_add();
    $display("A %0d %0d %0d", sbase::f.n, d.f.n, b.f.n);
    sderived::g = new; sd = new; ib = sd;
    sd.base_add(); sd.der_add();
    $display("B %0d %0d %0d", sderived::g.n, ib.g.n, sd.g.n);
  end
endmodule
"#
        ),
        [r#"A 1 11 1"#, r#"B 1 101 1"#]
    );
}
