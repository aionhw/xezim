//! Regression for factory type-override applied through `C::type_id::create`.
//!
//! A class `a` declares `typedef registry#(a,"a") type_id;`;
//! `a::type_id::create(name)` must run the registry's real `create` body
//! (which consults a type-override table and, when an override `a -> b` is
//! registered, constructs a `b`). Before the fix, xezim short-cut
//! `C::type_id::create` by directly constructing the requested type `C`,
//! bypassing the factory entirely — so the override was silently ignored and
//! the created object was an `a` instead of the overriding `b`, and the
//! factory's `used` counters stayed at 0.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const SRC: &str = r#"
module top;

  // A minimal factory: a type-override table mapping a requested type name to
  // the overriding type name.
  class factory;
    static string orig[$];
    static string ovrd[$];

    static function string find_override(string tname);
      for (int i = 0; i < orig.size(); i++) begin
        if (orig[i] == tname) return ovrd[i];
      end
      return tname;
    endfunction
  endclass

  class baseobj;
    string name;
    function new(string n = "obj");
      name = n;
    endfunction
    virtual function string kind();
      return "baseobj";
    endfunction
  endclass

  class a extends baseobj;
    typedef registry#(a, "a") type_id;
    function new(string n = "a");
      super.new(n);
    endfunction
    virtual function string kind();
      return "a";
    endfunction
  endclass

  class b extends a;
    typedef registry#(b, "b") type_id;
    function new(string n = "b");
      super.new(n);
    endfunction
    virtual function string kind();
      return "b";
    endfunction
  endclass

  // The registry whose `create` consults the override table and constructs
  // the overriding type when one is registered for the requested name `N`.
  // `C::type_id` typedefs to `registry#(C,...)`, so `C::type_id::create`
  // must land in this `create` body (not be bypassed by directly
  // constructing `C`).
  class registry#(type T = baseobj, string N = "x");
    typedef registry#(T, N) this_type;

    static function baseobj create(string name);
      string o = factory::find_override(N);
      if (o == "b") begin
        b handled = new(name);
        return handled;
      end
      T handled = new(name);
      return handled;
    endfunction
  endclass

  baseobj x;
  initial begin
    factory::orig.push_back("a");
    factory::ovrd.push_back("b");
    x = a::type_id::create("comp");
    $display("CREATED_KIND=%s", x.kind());
  end
endmodule
"#;

#[test]
fn type_id_create_routes_through_override_table() {
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("CREATED_KIND=b"),
        "a::type_id::create returned the requested type `a` instead of the \
         overriding `b` — the registry create (factory override) was bypassed:\n{}",
        out
    );
    assert!(
        !out.contains("CREATED_KIND=a"),
        "create bypassed the override table and constructed the requested type:\n{}",
        out
    );
}