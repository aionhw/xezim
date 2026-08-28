//! Static collection (associative-array) members of a PARAMETERIZED class,
//! accessed bare inside its static methods, must persist per-specialization.
//! UVM's config-db reuse depends on exactly this: `uvm_config_db` repeatedly
//! calls `uvm_config_db#(uvm_bitstream_t)::set(...)` for the SAME
//! field, expecting each subsequent call to REUSE the pool entry created on
//! the first call (moved to the head of the priority queue) rather than
//! inserting another resource. Without the fix every `set` re-created its
//! per-component `pool` (the static `m_rsc[uvm_component]` assoc-array member
//! could not be re-found afterward), so the resource queue grew 2,4,6,8,10,12
//! and the "expected 2, got N" check failed.
//!
//! This test mirrors that shape: a parameterized `config_db#(T)` holding a
//! static assoc array `m_rsc[comp]`, with `get_pool()` writing/reading it
//! BARE (no `this`, no explicit `#(spec)` prefix) inside the static method.
//! Five calls for one component and one for another must create exactly TWO
//! distinct pools — proving the static assoc member persists and is keyed
//! per-(class-specialization, component-object).
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/static_param_coll.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/static_param_coll.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"class comp;
endclass

class pool;
  static int instances;
endclass

class config_db #(type T = int);
  static pool m_rsc[comp];
  static function pool get_pool(comp cntxt);
    pool p;
    if(!m_rsc.exists(cntxt)) begin
      p = new;
      pool::instances = pool::instances + 1;
      m_rsc[cntxt] = p;
    end
    return m_rsc[cntxt];
  endfunction
endclass

module top;
  initial begin
    comp c;
    comp c2;
    c = new;
    c2 = new;
    for(int i=0; i<5; i++) begin
      void'(config_db#(int)::get_pool(c));
    end
    void'(config_db#(int)::get_pool(c2));
    // Five calls for one component + one for another => exactly TWO distinct
    // pools (one per component). Pre-fix each call created a fresh pool.
    if(pool::instances == 2)
      $display("TAG_PASS reuse, pools=%0d", pool::instances);
    else
      $display("TAG_FAIL pools=%0d", pool::instances);
    $finish;
  end
endmodule
"#;

#[test]
fn static_param_class_collection_reuses_shared_cell() {
    let out = run(SRC);
    assert!(
        out.contains("TAG_PASS reuse, pools=2"),
        "repeated get_pool(c) for the same component must reuse one static \
         assoc entry; expected pools=2, got:\n{out}"
    );
    assert!(
        !out.contains("TAG_FAIL"),
        "unexpected static-collection reuse failure:\n{out}"
    );
}