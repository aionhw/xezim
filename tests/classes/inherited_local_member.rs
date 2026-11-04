//! §8.18: a `local` member is visible only in its own class, so a derived
//! class's method cannot use an inherited one. The reference simulator
//! rejects that access and runs the legal classes with the same output.

use xezim::simulate;

#[test]
fn derived_class_uses_base_local() {
    for use_ in ["$display(a_loc);", "$display(this.a_loc);"] {
        let src = format!(
            "module top();\n  class a_cls;\n    local int a_loc = 21;\n    protected int a_prot = 22;\n  \
             endclass\n  class b_cls extends a_cls;\n    function void fun();\n      {use_}\n    \
             endfunction\n  endclass\n  b_cls b_obj;\n  initial begin\n    b_obj = new;\n    \
             b_obj.fun();\n  end\nendmodule\n"
        );
        assert!(simulate(&src, 10).is_err(), "accepted:\n{src}");
    }
}

#[test]
fn derived_class_uses_visible_members() {
    let src = r#"
module top();
  class a_cls;
    local int a_loc = 21;
    protected int a_prot = 22;
    int a = 23;
    function int get_loc(); return a_loc; endfunction
  endclass
  class b_cls extends a_cls;
    local int b_loc = 31;
    function void fun();
      int a_loc = 5;
      $display("L|%0d %0d %0d %0d %0d", a_prot, a, b_loc, a_loc, get_loc());
    endfunction
  endclass
  b_cls b_obj;
  initial begin
    b_obj = new;
    b_obj.fun();
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("visible members are legal");
    assert!(
        sim.output.iter().any(|o| o.message == "L|22 23 31 5 21"),
        "{:?}",
        sim.output
    );
}

/// The base class may come from a package or the compilation unit, found in
/// the map of classes every module sees, which the lint builds once per run
/// and shares with modules that declare no class of their own (#872).
#[test]
fn derived_class_uses_local_of_a_package_or_unit_base() {
    for (base, ext) in [
        (
            "package pk;\n  class a_cls;\n    local int a_loc = 21;\n  endclass\nendpackage\n",
            "pk::a_cls",
        ),
        ("class a_cls;\n  local int a_loc = 21;\nendclass\n", "a_cls"),
    ] {
        let src = format!(
            "{base}module other(); int x; endmodule\nmodule top();\n  other o();\n  \
             class b_cls extends {ext};\n    function void fun();\n      $display(a_loc);\n    \
             endfunction\n  endclass\n  b_cls b_obj;\n  initial begin\n    b_obj = new;\n    \
             b_obj.fun();\n  end\nendmodule\n"
        );
        assert!(simulate(&src, 10).is_err(), "accepted:\n{src}");
    }
}
