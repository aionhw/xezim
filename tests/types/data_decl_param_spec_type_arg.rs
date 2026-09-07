//! Regression: a data declaration of a parameterized CLASS whose specialization
//! argument is a TYPE KEYWORD — `param_obj #(int) m[string]`, `param_obj #(bit)
//! something` — must keep that `#(int)` so the variable/element binds the
//! concrete type parameter instead of the class default (`#(bit)`).
//!
//! `parse_identifier_starting_item` (the module-item parser for a bare
//! identifier as a type) parses the `#(...)` list into module-**instantiation**
//! `ParamConnection`s, then derives the data-declaration `type_args` by
//! filtering only `ParamValue::Expr` values. A type-keyword argument
//! (`int`, `bit`, …) is parsed by `parse_param_value` as `ParamValue::Type`,
//! which that filter dropped — so `param_obj #(int) x;` produced an empty
//! `type_args` list, the declaration default-specialized to `#(bit)`, and the
//! instance's per-view identity (`type_id::type_name()`) resolved to the wrong
//! specialization, breaking `get_object_type()` checks for every element built
//! from such a declaration.
//!
//! This repro builds a per-specialization static (`param_obj#(int)::bound`
//! versus `param_obj#(bit)::bound`) updated by the constructor via
//! `$typename(T)`, so constructing elements of `param_obj #(int) m[string]`
//! must set the `int` specialization's cell — not the `bit` one.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn data_decl_of_parameterized_class_keeps_type_keyword_spec_arg() {
    const SRC: &str = r#"
module top;
  class param_obj #(type T = bit);
    static string bound;   // per-specialization static cell
    function new();
      bound = $typename(T);
    endfunction
  endclass

  // The specialization arg is a TYPE KEYWORD (`int`) on an unpacked-collection
  // declaration. Elaboration records it in the collection's element type so a
  // later `m["a"] = new()` constructs the `int` specialization.
  param_obj #(int) intmap[string];
  param_obj #(bit) bitmap[string];

  initial begin
    intmap["a"] = new();
    bitmap["a"] = new();
    // If the declaration's #(int) is captured, intmap's element binds T=int and
    // the int specialization's constructor wrote param_obj#(int)::bound = "int".
    $display("BOUND int=[%s] bit=[%s]", param_obj#(int)::bound, param_obj#(bit)::bound);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("BOUND int=[int] bit=[bit]"),
        "the data-declaration #(int) specialization was dropped (intmap defaulted to #(bit)):\n{}",
        out
    );
}