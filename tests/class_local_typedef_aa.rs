//! Class-local typedef'd associative-array `ref` argument writeback.
//!
//! Companion to `ref_arg_assoc_writeback.rs`, isolating the *class-local
//! typedef* variant — the exact shape of UVM's phase-DAG successor edges:
//!
//! ```systemverilog
//!   class uvm_phase;
//!     typedef bit edges_t[uvm_phase];   // <-- typedef declared INSIDE the class
//!     ...
//!     function void get_successors(ref edges_t successors);
//!       foreach (m_successors[p]) successors[p] = 1;
//!     endfunction
//!   endfunction
//!   ...
//!   uvm_phase::edges_t edges;
//!   phase.get_successors(edges);   // ref fill — caller's array must grow
//! ```
//!
//! `port_is_assoc_array` detects an AA formal by looking up the typedef's
//! unpacked dimensions in `typedef_unpacked_dims`. But class-local typedefs
//! were never fed to `process_typedef` during elaboration (only module/package
//! typedefs were), so their dimensions were invisible and the `ref edges_t`
//! formal was mis-bound as a scalar — the writeback was silently lost, the
//! phase DAG never expanded, and every UVM test stalled at time 0.
//!
//! The fix registers class-local typedefs in `register_class_enum_members`.
//! Verified byte-for-byte against reference simulators.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_pass(sim: &xezim::compiler::Simulator, tag: &str) {
    let msgs = messages(sim);
    let pass = msgs.iter().any(|m| m.contains(&format!("{tag}_PASS")));
    let fail = msgs.iter().find(|m| m.contains(&format!("{tag}_FAIL")));
    assert!(
        pass,
        "expected {tag}_PASS in output\nfail line: {fail:?}\nfull output: {msgs:?}"
    );
}

/// The exact UVM `edges_t` shape: a typedef'd AA declared inside a class, used
/// as a `ref` formal in a method, caller's actual under a different name.
/// Pre-fix the caller's array stayed empty (num==0).
const CLASS_LOCAL_TYPEDEF_AA: &str = r#"
class Node;
  function new(string n); endfunction
endclass
class Ph;
  typedef bit edges_t[Node];          // class-local typedef (like uvm_phase::edges_t)
  protected edges_t m_succ;
  function void add_edge(input Node n); m_succ[n] = 1; endfunction
  function void get_successors(ref edges_t successors);
    foreach (m_succ[p]) successors[p] = 1;
  endfunction
  // Caller-side helper so the typedef'd type is used in-class.
  function void check(output int n);
    edges_t e;
    get_successors(e);
    n = e.num();
  endfunction
endclass
module top;
  initial begin
    int n;
    Ph h;
    Node a; Node b;
    h = new;
    a = new("A");
    b = new("B");
    h.add_edge(a);
    h.add_edge(b);
    h.check(n);
    if (n == 2) $display("CLTAA_PASS num=%0d", n);
    else        $display("CLTAA_FAIL num=%0d", n);
  end
endmodule
"#;

#[test]
fn class_local_typedef_aa_ref_writeback() {
    let sim = simulate(CLASS_LOCAL_TYPEDEF_AA, 100).expect("simulate failed");
    assert_pass(&sim, "CLTAA");
}
