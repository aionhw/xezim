//! A VALUE PARAMETER of an enclosing generic class (e.g. `string FIELD`) that
//! is passed as an ACTUAL ARGUMENT into a method of a DIFFERENT parameterized
//! class must resolve to its bound value, not its declared default. §13.5.1
//! evaluates actual arguments in the CALLER's scope.
//!
//! UVM's `uvm_utils #(type TYPE=int, string FIELD="config")` does exactly this:
//! `get_config` calls `m_uvm_config_obj_misc::get(comp, "", FIELD, obj)`
//! (`uvm_config_db#(uvm_object)::get`), so a `uvm_utils #(test_object, "cfg")`
//! passed `FIELD="cfg"` as the resource field name. xezim switched
//! `current_spec` to the CALLEE (`uvm_config_db#(uvm_object)`) before binding
//! the args, so `FIELD` resolved against the callee's params — not found,
//! fell back to the declared default `"config"` — and the config lookup used
//! the wrong field name, missing the entry that `set_config_object` made.
//!
//! Fix: keep the enclosing specializations on a scope stack (`spec_scope_stack`
//! pushed by `eval_call` and the static-dispatch sites) so value-param
//! resolution can walk up to the caller. Verified byte-for-byte against a
//! commercial simulator: `VP_PASS got=[cfg]`.

use xezim::simulate_multi;

#[test]
fn t_bound_generic_value_param_argument() {
    let src = r#"
module top;
  // A generic class whose method passes its `string FIELD` VALUE PARAMETER as
  // an argument into a method of ANOTHER parameterized class. The callee
  // stored the field name it received so the caller can verify the bound
  // value ("cfg") arrived — not the parameter's declared default ("config").
  class Sink #(type T=int);
    static string got;
    static function void note(string x); got = x; endfunction
  endclass
  class Util #(type TYPE=int, string FIELD="config");
    static function void go();
      Sink #(int)::note(FIELD);
    endfunction
  endclass
  initial begin
    Util #(int, "cfg")::go();
    if (Sink #(int)::got == "cfg") $display("VP_PASS got=[%s]", Sink #(int)::got);
    else $display("VP_FAIL got=[%s]", Sink #(int)::got);
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
        out.iter().any(|l| l == "VP_PASS got=[cfg]"),
        "bound value param must reach the callee, not its default; got {:?}", out
    );
    assert!(
        !out.iter().any(|l| l.starts_with("VP_FAIL")),
        "default leaked through; got {:?}", out
    );
}