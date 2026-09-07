//! Regression for parameterized class registry `type_id` identity and
//! `$cast` through typedef aliases (mirrors the UVM factory/registry
//! parameterized-object test, where the typedefs live at class scope and the
//! checks run inside a class method):
//!
//! - `BaseType::type_id::get()` (a class-scoped member-typedef static call
//!   reached through a parameterized-class typedef alias) must return the
//!   SAME per-specialization singleton as `RegistryType::get()`.
//! - `$cast` to a destination declared via a typedef alias to a parameterized
//!   registry must not reject the cast because the alias's `#(...)` args were
//!   dropped (the type-param comparison then compared a base name against the
//!   instance's concrete specialization and wrongly failed).
//! - The concrete registry specialization (here `oreggy#(base_class#(9))`)
//!   must register when the concrete parameterized class (`base_class#(9)`) is
//!   used via a typedef, not just the default `#(0)`.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const SRC: &str = r#"
class base_class#( int P=0 );
  typedef oreggy#(base_class#(P)) type_id;
  static function type_id get_type();
    return type_id::get();
  endfunction
endclass
class derived_class#( int P=0 ) extends base_class#(P);
  typedef oreggy#(derived_class#(P)) type_id;
endclass

class oreggy#( type T=base_class#(0) );
  static function oreggy#(T) get();
    static oreggy#(T) m_inst;
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
endclass

// A minimal factory: a static queue of registered type_id handles. Mirrors
// how UVM's factory registers a wrapper singleton per specialization.
class factory;
  static oreggy registered[$];

  static function void do_register(oreggy w);
    registered.push_back(w);
  endfunction
  static function bit is_registered(oreggy w);
    for (int i=0;i<registered.size();i++) begin
      if (registered[i] === w) return 1;
    end
    return 0;
  endfunction
endclass

class test;
  typedef base_class#( 9 )        BaseType;
  typedef derived_class#( 9 )     DerivedType;
  typedef oreggy#( BaseType )     RegistryType;

  task run_phase();
    RegistryType base_regtry_inst;
    RegistryType via_type_id;
    // 1. type_id::get() via a member typedef reached through a parameterized
    //    class typedef alias must equal RegistryType::get().
    base_regtry_inst = RegistryType::get();
    via_type_id = BaseType::type_id::get();
    $display("TYPEID_SAME %0d", (base_regtry_inst!==null && base_regtry_inst===via_type_id));

    // 2. $cast to a typedef-alias-typed parameterized registry destination
    //    must succeed (the resolved `#(args)` must survive the alias).
    base_regtry_inst = null;
    if ($cast(base_regtry_inst, via_type_id))
      $display("CAST_ALIAS_OK"); else $display("CAST_ALIAS_FAIL");

    // 3. The concrete #(9) specialization registers with the factory.
    factory::do_register(BaseType::type_id::get());
    $display("REGISTERED %0d", factory::is_registered(RegistryType::get()));
  endtask
endclass

module top;
  test t;
  initial begin
    t = new();
    t.run_phase();
  end
endmodule
"#;

#[test]
fn param_registry_typeid_identity_and_cast_and_registration() {
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("TYPEID_SAME 1"),
        "BaseType::type_id::get() != RegistryType::get() (per-spec singleton mismatch):\n{}",
        out
    );
    assert!(
        out.contains("CAST_ALIAS_OK"),
        "$cast to typedef-alias parameterized registry destination was rejected:\n{}",
        out
    );
    assert!(
        out.contains("REGISTERED 1"),
        "concrete parameterized registry specialization #(9) not registered:\n{}",
        out
    );
}