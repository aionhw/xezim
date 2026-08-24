//! A parameterless STATIC function of a class invoked BY NAME WITHOUT
//! PARENTHESES must be CALLED, not read as a (non-existent) static property
//! (LRM §13.4.1: a no-arg function may be invoked by name alone). UVM depends
//! on this in `uvm_misc.svh`'s `find()`/`find_all()` warning:
//!
//!   `uvm_warning("find_type-multi match",
//!     {"More than one instance of type '", TYPE::type_name, " found ..."})
//!
//! where `TYPE` is a class type-parameter and `type_name` is
//! `static function string type_name()`. In a 7443 compat test the parameter
//! was bound to `comp_b`, and xezim rendered the warning's type name EMPTY
//! (`type ' found`) instead of the reference's `type 'comp_b found`. The flat
//! 2-segment `comp_b::type_name` and the MemberAccess form `TYPE::type_name`
//! both fell through `class_static_get` (which only reads static PROPERTIES),
//! so the call returned the unused property cell (empty) instead of invoking
//! `comp_b::type_name()`.
//!
//! Fix: at the `ClassName::x` static read, if `x` is a parameterless static
//! function, dispatch it via `exec_static_method` (mirrored in the Ident arm
//! and the MemberAccess arm). Verified byte-for-byte against a commercial
//! simulator: `'comp_b found`.

use xezim::simulate_multi;

#[test]
fn t_no_parens_static_method_invocation() {
    let src = r#"
module top;
  class comp_b;
    static function string type_name(); return "comp_b"; endfunction
  endclass
  class Util #(type TYPE=comp_b);
    static function string msg();
      return {"More than one instance of type '", TYPE::type_name, " found"};
    endfunction
    static function void go();
      $display("TYN11 '%s'", msg());
    endfunction
  endclass
  // Also exercise the flat (no type-param) direct-reference forms.
  initial begin
    Util #(comp_b)::go();
    $display("TYN11 direct ['%s']", comp_b::type_name);
  end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()], 1000, Some("top"), &[], &[], None, false, None, None,
        &[], &[], 1, None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    assert!(
        out.iter().any(|l| l == "TYN11 'More than one instance of type 'comp_b found'"),
        "no-parens static method must be invoked (type name resolved); got {:?}", out
    );
    assert!(
        out.iter().any(|l| l == "TYN11 direct ['comp_b']"),
        "direct Class::method no-parens must call the method; got {:?}", out
    );
}