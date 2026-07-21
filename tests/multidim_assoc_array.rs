//! IEEE 1800-2023 §7.8.4 — multidimensional associative arrays
//! (`T m[K1][K2]...[Kn]`, an assoc array whose elements are themselves assoc
//! arrays) must store/recover elements, and `.exists()`/`.num()` must reflect
//! each dimension.
//!
//! Root cause this guards: every element of a multidim assoc was silently
//! lost. The declaration handlers registered only the FIRST unpacked
//! dimension (so `m` looked like a 1D assoc), and neither the read nor write
//! `ExprKind::Index` path had an arm for a base that is itself an
//! associative-array index. `m[k1][k2] = v` fell through every handler and
//! stored nothing; `m[k1][k2]` read 0; `m.exists(k1)` (which checks the bare
//! `m[k1]` key, never populated in 2D) returned 0. This broke UVM's printer
//! cycle-detection map `m_recur_states[uvm_object][uvm_recursion_policy_enum]`
//! — `m[obj][policy] = STARTED` never landed, so revisiting an object during
//! `sprint()` was never detected and a circular reference recursed to stack
//! overflow (UVM Mantis 8522).
//!
//! The fix stores each element under the flat compound key `m[K1][K2]...[Kn]`
//! (read and write share one helper so the key rendering agrees for every key
//! type), and `.exists(k)` accepts either the 1D key OR any `m[k][...]`
//! compound element (prefix scan). These tests pin the semantics for
//! module-level and class-member multidim assocs.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("top.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// Module-level `int m[int][int]`: write, read back, `.num()` on an inner
/// sub-array, and `.exists()` on the outer dimension.
#[test]
fn multidim_assoc_module_level_write_read_exists_num() {
    let src = r#"
`timescale 1ns/1ns
module top;
  int m [int][int];
  int result = 0;
  initial begin
    m[1][0] = 100;
    m[1][1] = 101;
    m[2][0] = 200;
    // read-back: 100+101+200 = 401
    result = m[1][0] + m[1][1] + m[2][0];
    // inner .num(): m[1] has two entries → +1000
    if (m[1].num() == 2) result = result + 1000;
    // outer .exists(): +10000 if m[2] present, +100000 if m[9] absent
    if (m.exists(2))  result = result + 10000;
    if (!m.exists(9)) result = result + 100000;
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    // 401 + 1000 + 10000 + 100000 = 111401
    assert_eq!(
        lookup(&sim, "result"),
        111401,
        "2D assoc write/read/num/exists must all work at module scope"
    );
}

/// A class-handle-indexed 2D assoc (`int m[C][int]`) must key on handle
/// identity: two distinct handles get distinct entries; an alias handle reads
/// the same entry.
#[test]
fn multidim_assoc_handle_keyed() {
    let src = r#"
`timescale 1ns/1ns
class C;
  int id;
  function new(int i); id = i; endfunction
endclass
module top;
  int m [C][int];
  int result = 0;
  initial begin
    C c1 = new(1);
    C c2 = new(2);
    C c1b; c1b = c1;   // alias of c1
    m[c1][10] = 7;
    m[c2][10] = 8;
    result = m[c1][10] + m[c2][10] + m[c1b][10];  // 7 + 8 + 7 = 22
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "result"),
        22,
        "handle-keyed 2D assoc must key on identity (alias reads same entry)"
    );
}

/// A multidim assoc declared as a CLASS member (mirrors
/// `uvm_printer::m_recur_states[obj][policy]`): writes/reads/exists through
/// methods must resolve to the per-instance storage.
#[test]
fn multidim_assoc_class_member() {
    let src = r#"
`timescale 1ns/1ns
class C;
  int id;
  function new(int i); id = i; endfunction
endclass
class Holder;
  int m [C][int];
  function void put(input C k, input int p, input int v); m[k][p] = v; endfunction
  function int  get(input C k, input int p); return m[k][p]; endfunction
  function int  has(input C k, input int p); return m[k].exists(p); endfunction
endclass
module top;
  int result = 0;
  initial begin
    Holder h = new();
    C c1 = new(1);
    h.put(c1, 0,     100);
    h.put(c1, 65536, 200);
    result = h.get(c1, 0) + h.get(c1, 65536);   // 300
    if (h.has(c1, 0) && h.has(c1, 65536)) result = result + 1000;   // 1300
    if (!h.has(c1, 99)) result = result + 10000;                    // 11300
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "result"),
        11300,
        "class-member 2D assoc write/read/exists through methods"
    );
}

/// `.delete()` on a 2D assoc clears every compound element (prefix scan),
/// so a subsequent `.exists()` returns 0.
#[test]
fn multidim_assoc_delete_clears_all() {
    let src = r#"
`timescale 1ns/1ns
module top;
  int m [int][int];
  int result = 0;
  initial begin
    m[1][2] = 9;
    m[3][4] = 8;
    result = m.exists(1);   // 1 before delete
    m.delete();
    result = result*10 + m.exists(1);  // 10 (exists now 0)
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "result"),
        10,
        "delete() must clear all compound elements of a 2D assoc"
    );
}
