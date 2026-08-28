//! An iterator/generic instantiated with the ENCLOSING parameterized class's
//! OWN value parameter as a nested type argument must have that parameter
//! SUBSTITUTED to the concrete value at construction — not captured as the
//! bare `#(N)` text. UVM's `uvm_callback_iter#(special_comp#(N),
//! special_cb#(N)) iter = new(this)` inside `special_comp#(N)` binds the
//! iterator's type param `T` to the enclosing specialization; if the binding
//! carried the literal `N` instead of `special_comp#(2)`, then
//! `iter.first()` (calling `uvm_callbacks#(T,CB)::get_first`) resolved T to
//! the bare `special_comp` and looked up the UN-specialized typewide queue —
//! empty — so the parameterized special_cb callbacks never fired and the
//! whole params test failed its a1/a2 callback-sequence
//! checks. This mirrors that shape without the UVM package: a method of the
//! parameterized class builds an iterator typed `iter#(comp#(N), cb#(N))`
//! and probes a per-type registry through T, expecting per-instance spec
//! state.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/param_type_binding_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const PARAM_TYPE_BINDING: &str = r#"module top;
  class base_comp;
    function new(); endfunction
  endclass
  class cb #(int N=0);
    function new(); endfunction
  endclass
  // Per-type (and per-spec) global store, like `callbacks#(T,CB)`
  // `m_tw_cb_q`. The registry's spec string lives in a per-spec static.
  class registry #(type T=base_comp, type CB=cb#(0));
    static string m_cell;
    static function void put(string s);
      m_cell = s;
    endfunction
    static function string get();
      return (m_cell == "") ? "UNSET" : m_cell;
    endfunction
  endclass
  class iter #(type T=base_comp, type CB=cb#(0));
    local T m_obj;
    function new(T obj); m_obj=obj; endfunction
    function string lookup();
      return registry#(T, CB)::get();
    endfunction
  endclass
  class comp #(int N=0) extends base_comp;
    int mark;
    function new(); mark=0; endfunction
    // register this specialization in its own registry cell
    function void set_reg();
      registry#(comp#(N), cb#(N))::put($sformatf("comp#(%0d)", N));
    endfunction
    // build an iterator typed with comp#(N) and CB#(N) and probe per-spec state
    function string probe();
      iter#(comp#(N), cb#(N)) it;
      it = new(this);
      return it.lookup();
    endfunction
  endclass
  initial begin
    comp#(1) c1;
    comp#(2) c2;
    string s1, s2;
    c1 = new;
    c2 = new;
    c1.set_reg();
    c2.set_reg();
    s1 = c1.probe();
    s2 = c2.probe();
    $display("RESULT comp#(1) probe=%s", s1);
    $display("RESULT comp#(2) probe=%s", s2);
    if (s1 == "comp#(1)") $display("RESULT PASS c1_own_spec");
    else                  $display("RESULT FAIL c1_probe");
    if (s2 == "comp#(2)") $display("RESULT PASS c2_own_spec");
    else                  $display("RESULT FAIL c2_probe");
    if (s1 == "comp#(1)" && s2 == "comp#(2)")
      $display("RESULT PASS both_per_spec");
    else
      $display("RESULT FAIL bare_N_or_shared_cell");
    #1; $finish;
  end
endmodule
"#;

#[test]
fn param_type_binding_resolves_enclosing_value_param_for_nested_generic() {
    let out = run(PARAM_TYPE_BINDING, "binding");
    assert!(
        out.contains("RESULT PASS both_per_spec"),
        "an iterator typed `iter#(comp#(N), cb#(N))` built inside `comp#(N)` must\n\
         bind the CONCRETE N, so the per-type registry (looked up through T)\n\
         answers each instance's own specialization:\n{out}"
    );
    assert!(
        out.contains("RESULT comp#(1) probe=comp#(1)"),
        "comp#(1)'s iterator must resolve comp#(1), not a bare-N/un-specialized cell:\n{out}"
    );
    assert!(
        out.contains("RESULT comp#(2) probe=comp#(2)"),
        "comp#(2)'s iterator must resolve comp#(2), not leak comp#(1):\n{out}"
    );
}