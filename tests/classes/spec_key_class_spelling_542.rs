//! IEEE 1800 §8.25/§6.18/§8.23: a parameterized class has ONE specialization
//! per argument TYPE, however that type is spelled — bare through a wildcard
//! import (`Cfg`), package-qualified (`my_pkg::Cfg`), through a module- or
//! class-scope typedef, or as a nested class (`Outer::Inner` vs
//! `my_pkg::Outer::Inner`). Every specialization carries its own statics, so
//! a spelling split silently splits the statics: the `uvm_config_db#(T)`
//! set/get miss of the dram_bfm_sat regression is the user-visible shape.
//! Distinct classes that merely share a leaf name must stay distinct.

fn lines(src: &str, top: &str) -> Vec<String> {
    xezim::simulate_multi(
        &[src.to_string()],
        1000,
        Some(top),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

const BOX: &str = r#"
package util_pkg;
  class box #(type T = int);
    static T stored;
    static function void put(T t);
      stored = t;
    endfunction
    static function bit get(output T t);
      t = stored;
      return (stored != null);
    endfunction
  endclass
endpackage
"#;

/// `box#(Cfg)` and `box#(my_pkg::Cfg)` share one `stored` static.
#[test]
fn spec_key_bare_vs_qualified_share_statics() {
    let src = format!(
        r#"package my_pkg;
  class Cfg;
    int v;
    function new(); v = 42; endfunction
  endclass
endpackage
{BOX}
module tb;
  import my_pkg::*;
  import util_pkg::*;
  Cfg c;
  initial begin
    Cfg obj = new();
    box#(Cfg)::put(obj);
    #1;
    if (box#(my_pkg::Cfg)::get(c)) $display("T|bare_qual %0d", c.v);
    else                           $display("T|bare_qual FAIL");
    box#(my_pkg::Cfg)::put(obj);
    #1;
    if (box#(Cfg)::get(c))         $display("T|qual_bare %0d", c.v);
    else                           $display("T|qual_bare FAIL");
  end
endmodule"#
    );
    let got = lines(&src, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>();
    assert_eq!(got, ["T|bare_qual 42", "T|qual_bare 42"]);
}

/// Module-scope typedefs of the class — qualified or bare target — key the
/// same specialization as a direct naming (§6.18: a typedef is a synonym).
#[test]
fn spec_key_typedef_spellings_share_statics() {
    let src = format!(
        r#"package my_pkg;
  class Cfg;
    int v;
    function new(); v = 42; endfunction
  endclass
endpackage
{BOX}
module tb;
  import my_pkg::*;
  import util_pkg::*;
  typedef my_pkg::Cfg CfgTQ;
  typedef Cfg CfgTB;
  Cfg c;
  initial begin
    Cfg obj = new();
    box#(CfgTQ)::put(obj);
    #1;
    if (box#(Cfg)::get(c))         $display("T|typedefq %0d", c.v);
    else                           $display("T|typedefq FAIL");
    box#(CfgTB)::put(obj);
    #1;
    if (box#(my_pkg::Cfg)::get(c)) $display("T|typedefb %0d", c.v);
    else                           $display("T|typedefb FAIL");
  end
endmodule"#
    );
    let got = lines(&src, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>();
    assert_eq!(got, ["T|typedefq 42", "T|typedefb 42"]);
}

/// A class-local typedef (`typedef Cfg cfg_t;`) used as a type argument.
#[test]
fn spec_key_class_local_typedef_shares_statics() {
    let src = format!(
        r#"package my_pkg;
  class Cfg;
    int v;
    function new(); v = 42; endfunction
  endclass
endpackage
{BOX}
module tb;
  import my_pkg::*;
  import util_pkg::*;
  class user;
    typedef Cfg cfg_t;
    static function void do_put(Cfg o);
      box#(cfg_t)::put(o);
    endfunction
  endclass
  Cfg c;
  initial begin
    Cfg o = new();
    user::do_put(o);
    #1;
    if (box#(Cfg)::get(c)) $display("T|cltypedef %0d", c.v);
    else                   $display("T|cltypedef FAIL");
  end
endmodule"#
    );
    let got = lines(&src, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>();
    assert_eq!(got, ["T|cltypedef 42"]);
}

/// A nested class spelled `Outer::Inner` and `my_pkg::Outer::Inner` is one
/// type (§8.23); a DIFFERENT package's `Wrapper::Inner` is not.
#[test]
fn spec_key_nested_class_spellings_share_statics() {
    let src = format!(
        r#"package my_pkg;
  class Outer;
    class Inner;
      int v;
      function new(); v = 7; endfunction
    endclass
  endclass
endpackage
package other_pkg;
  class Wrapper;
    class Inner;
      int v;
      function new(); v = 9; endfunction
    endclass
  endclass
endpackage
{BOX}
module tb;
  import my_pkg::*;
  import util_pkg::*;
  Outer::Inner c;
  other_pkg::Wrapper::Inner oc;
  initial begin
    Outer::Inner obj = new();
    box#(Outer::Inner)::put(obj);
    #1;
    if (box#(my_pkg::Outer::Inner)::get(c)) $display("T|nested %0d", c.v);
    else                                    $display("T|nested FAIL");
    // A different package's same-leaf nested class is a DIFFERENT type:
    // its box cell must still be empty.
    if (box#(other_pkg::Wrapper::Inner)::get(oc)) $display("T|crossed %0d", oc.v);
    else                                          $display("T|crossed EMPTY");
  end
endmodule"#
    );
    let got = lines(&src, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>();
    assert_eq!(got, ["T|nested 7", "T|crossed EMPTY"]);
}

/// Two packages each declaring `Cfg` are two types: the specializations —
/// and their statics — must not merge across them.
#[test]
fn spec_key_distinct_same_leaf_classes_stay_distinct() {
    let src = format!(
        r#"package pkg_a;
  class Cfg;
    int v;
    function new(); v = 42; endfunction
  endclass
endpackage
package pkg_b;
  class Cfg;
    int v;
    function new(); v = 13; endfunction
  endclass
endpackage
{BOX}
module tb;
  import util_pkg::*;
  pkg_a::Cfg ca;
  pkg_b::Cfg cb;
  initial begin
    pkg_a::Cfg a = new();
    box#(pkg_a::Cfg)::put(a);
    #1;
    if (box#(pkg_a::Cfg)::get(ca)) $display("T|a_still %0d", ca.v);
    else                           $display("T|a_still FAIL");
    if (box#(pkg_b::Cfg)::get(cb)) $display("T|b_crossed %0d", cb.v);
    else                           $display("T|b_crossed EMPTY");
  end
endmodule"#
    );
    let got = lines(&src, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>();
    assert_eq!(got, ["T|a_still 42", "T|b_crossed EMPTY"]);
}