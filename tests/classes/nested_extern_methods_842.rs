//! IEEE 1800-2023 §8.23/§8.24 (#842): nested extern method definitions
//! inside the enclosing class execute their own bodies and retain qualifiers.

use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 100).expect("simulate");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(!lines.iter().any(|m| m.starts_with("FAIL")), "{lines:#?}");
    assert_eq!(lines.last().copied(), Some("ALL PASS"), "{lines:#?}");
}

#[test]
fn nested_extern_methods_in_enclosing_class() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    check(include_str!("nested_extern_methods_842.sv"));
    check(include_str!("nested_extern_variants_842.sv"));
    check(include_str!("nested_extern_clash_842.sv"));
}

#[test]
fn nested_and_outer_names_stay_distinct_in_either_source_order() {
    let source = include_str!("nested_extern_clash_842.sv");
    let own = "  function int describe();             // the outer class own method, first\n    return 200 + tag;\n  endfunction\n";
    assert!(source.contains(own));
    let reordered = source.replacen(own, "", 1).replacen(
        "endclass\n\nmodule t;",
        &format!("{own}endclass\n\nmodule t;"),
        1,
    );
    check(&reordered);
}

#[test]
fn package_homonyms_run_their_own_nested_bodies() {
    check(
        r#"
package p;
  class outer;
    class inner;
      int v;
      extern function new(int x);
      extern function int get();
    endclass
    function inner::new(int x); v=x+10; endfunction
    function int inner::get(); return v; endfunction
  endclass
endpackage
package q;
  class outer;
    class inner;
      int v;
      extern function new(int x);
      extern function int get();
    endclass
    function inner::new(int x); v=x+20; endfunction
    function int inner::get(); return v; endfunction
  endclass
endpackage
module t;
  initial begin
    p::outer::inner a = new(1);
    q::outer::inner b = new(2);
    if (a.get() != 11 || b.get() != 22) $display("FAIL package method identity");
    else $display("ALL PASS");
  end
endmodule
"#,
    );
}

#[test]
fn parameterized_nested_class_keeps_qualified_identity() {
    check(
        r#"
class inner; int unrelated; endclass
package pk;
  class outer #(type T=int);
    class inner;
      T v;
      extern function T get();
    endclass
    function T inner::get(); return v; endfunction
  endclass
endpackage
module t;
  initial begin
    pk::outer#(int)::inner a = new();
    pk::outer#(byte)::inner b = new();
    a.v = 11;
    b.v = -127;
    if (a == null || b == null) $display("FAIL nested construction");
    else if (a.get() !== 11 || b.get() !== -127)
      $display("FAIL nested type binding");
    else if ($bits(b.v) != 8) $display("FAIL nested property width");
    else $display("ALL PASS");
  end
endmodule
"#,
    );
}

#[test]
fn nested_specialization_keeps_outer_and_inner_type_arguments() {
    check(
        r#"
class inner; int unrelated; endclass
package pk;
  class outer #(type T=int, type PAD=int);
    class inner #(type U=int);
      T first;
      U second;
      extern function T read_first();
      extern function U read_second();
    endclass
    function T inner::read_first(); return first; endfunction
    function U inner::read_second(); return second; endfunction
  endclass
endpackage
module t;
  initial begin
    pk::outer#(byte)::inner#(shortint) a = new();
    pk::outer#(.T(shortint))::inner#(.U(byte)) b;
    b = new();
    a.first = -127; a.second = -30000;
    b.first = -30000; b.second = -100;
    if (a == null || b == null) $display("FAIL nested construction");
    else if (a.read_first() !== -127 || a.read_second() !== -30000
             || b.read_first() !== -30000 || b.read_second() !== -100)
      $display("FAIL nested independent argument binding");
    else if ($bits(a.first) != 8 || $bits(a.second) != 16
             || $bits(b.first) != 16 || $bits(b.second) != 8)
      $display("FAIL nested independent property widths");
    else $display("ALL PASS");
  end
endmodule
"#,
    );
}
