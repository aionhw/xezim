//! UVM callback infrastructure: typedef-scoped static method calls and
//! per-spec static keying in parameterized class hierarchies.
//!
//! Two bugs that together broke UVM's entire callback subsystem:
//!
//! 1. `super_type::method()` — a typedef-resolved class scope on a static
//!    method call was silently dropped. UVM's callback classes declare
//!    `typedef base super_type;` and call `super_type::m_initialize()` to
//!    set up the shared static pool — without this resolution, the pool
//!    stayed null and no callback was ever stored or retrieved.
//!
//! 2. Static fields inherited from a parameterized ancestor were keyed
//!    per-leaf-spec instead of per-declaring-class-spec. UVM's
//!    `uvm_typed_callbacks#(T)::m_tw_cb_q` was stored under different keys
//!    depending on whether it was accessed from `uvm_callbacks#(T, CB)`
//!    or `uvm_callbacks#(T, uvm_callback)` — so writes and reads missed
//!    each other.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── typedef-resolved static call (§6.18/§8.25.1) ──────────────────────
const TYPEDEF_STATIC_SRC: &str = r#"
module top;
  class base;
    static int val = -1;
    static function void init();
      if (val == -1) val = 42;
    endfunction
  endclass
  class sub #(type T = int) extends base;
    typedef base super_type;
    static function void go();
      super_type::init();
    endfunction
  endclass
  initial begin
    sub#(int)::go();
    if (base::val == 42) $display("TAG_PASS");
    else $display("TAG_FAIL val=%0d", base::val);
  end
endmodule
"#;

#[test]
fn typedef_resolved_static_call() {
    let sim = simulate(TYPEDEF_STATIC_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "super_type::init() should resolve to base::init(); got {:?}",
        msgs
    );
}

// ── inherited static keying in parameterized hierarchy ────────────────
// typed_cb#(T) has a static; cb#(T,CB) extends it and reassigns m_t_inst.
// The static must survive the reassignment because it's class-level.
const INHERITED_STATIC_SRC: &str = r#"
module top;
  class typed_cb #(type T = int);
    static int s_count = -1;
    static typed_cb#(T) m_t_inst;
    static function typed_cb#(T) m_init();
      if (m_t_inst == null) begin
        m_t_inst = new;
        m_t_inst.s_count = 42;
      end
      return m_t_inst;
    endfunction
  endclass
  class cb #(type T = int, type CB = int) extends typed_cb#(T);
    static cb#(T,CB) m_inst;
    static function cb#(T,CB) get();
      if (m_inst == null) begin
        void'(m_init());
        m_inst = new;
        m_t_inst = m_inst;
      end
      return m_inst;
    endfunction
  endclass
  initial begin
    void'(cb#(int, byte)::get());
    if (typed_cb#(int)::s_count == 42) $display("TAG_PASS");
    else $display("TAG_FAIL count=%0d", typed_cb#(int)::s_count);
  end
endmodule
"#;

#[test]
fn inherited_static_param_hierarchy() {
    let sim = simulate(INHERITED_STATIC_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "inherited static must persist across m_t_inst reassignment; got {:?}",
        msgs
    );
}
