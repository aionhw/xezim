//! §26.3: package data declarations (`const` and plain variables) referenced
//! with `pkg::X` from inside a SUBROUTINE body (function/task/class method).
//!
//! The elaborator collapses `pkg::X` into a flat two-segment `Ident` for
//! always/initial/continuous-assign code (where it resolves via the bare-name
//! signal/parameter read), but inside a subroutine body the reference arrives
//! as `MemberAccess(Ident(["pkg"]), "X")`. That arm resolved package ENUM
//! members and package PARAMETER/localparams but had no case for a package
//! DATA declaration — a `const` or a plain variable — which are registered
//! under their bare name as a signal. Those fell through to an object-property
//! read returning 0, so `pkg::MYCONST` (a `const int`) read 0 inside a
//! function while the same reference at module scope read its true value.
//!
//! The const, variable, and parameter cases are cross-checked against an
//! independent tool's output, including the shadowing rule: an explicit
//! `pkg::X` qualification is NOT shadowed by a same-named subroutine local.

use xezim::simulate;

#[test]
fn package_data_members_resolve_inside_subroutines() {
    let src = r#"
package pkg;
  const int CONSTC = 42;
  int PLAINV = 100;          // package variable
  parameter int PARAMC = 7;  // parameter (already worked)
endpackage

module top;
  function int get_const();
    return pkg::CONSTC;
  endfunction
  function int get_var();
    return pkg::PLAINV;
  endfunction
  function int get_param();
    return pkg::PARAMC;
  endfunction

  class C;
    function int get_const_m();
      return pkg::CONSTC;
    endfunction
  endclass

  initial begin
    C c = new();
    $display("RESULT const_fn=%0d", get_const());      // 42
    $display("RESULT var_fn=%0d", get_var());          // 100
    $display("RESULT param_fn=%0d", get_param());      // 7
    $display("RESULT const_m=%0d", c.get_const_m());   // 42, class method
    $display("RESULT const_top=%0d", pkg::CONSTC);     // 42 (module scope)
    $display("RESULT var_top=%0d", pkg::PLAINV);       // 100
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs: Vec<String> =
        sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "RESULT const_fn=42"),
        "package const must read 42 inside a function; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT var_fn=100"),
        "package variable must read 100 inside a function; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT param_fn=7"),
        "package parameter inside a function; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT const_m=42"),
        "package const inside a class method; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT const_top=42"),
        "module-scope reference unchanged; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT var_top=100"),
        "module-scope variable unchanged; got {:?}", msgs
    );
}

#[test]
fn package_qualification_is_not_shadowed_by_subroutine_local() {
    let src = r#"
package pkg;
  const int CONSTC = 42;
  int PLAINV = 100;
endpackage

module top;
  function int get_const_shadow();
    int CONSTC;              // local shadows the bare name
    CONSTC = 999;
    return pkg::CONSTC;      // must still be 42 (qualified)
  endfunction
  function int get_var_shadow();
    int PLAINV;
    PLAINV = 999;
    return pkg::PLAINV;      // must still be 100 (qualified)
  endfunction
  initial begin
    $display("RESULT const_shadow=%0d", get_const_shadow());
    $display("RESULT var_shadow=%0d", get_var_shadow());
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs: Vec<String> =
        sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "RESULT const_shadow=42"),
        "explicit pkg::const bypasses a same-named local; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT var_shadow=100"),
        "explicit pkg::var bypasses a same-named local; got {:?}", msgs
    );
}