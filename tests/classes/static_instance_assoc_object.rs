//! Static associative-array members (of OBJECT handles) of a PARAMETERIZED
//! class, accessed BARE from an INSTANCE (non-static) method, must persist to
//! the shared per-specialization store — the same cell a static method and a
//! `ClassName::m[...]` spelling use. UVM `uvm_config_db`'s `wait_modified`
//! waiter map (`static local uvm_queue m_waiters[string]` in
//! `uvm_config_db_default_implementation_t#(T)`) is exactly this shape: each
//! component's waiter task runs as an INSTANCE method that registers a waiter
//! into the shared `m_waiters[field]` object and later reads `exists(field)`.
//!
//! Pre-fix, a bare read of a passed assoc-object value from an instance method
//! of a parameterized class resolved to a DIFFERENT (fresh empty) store than the
//! write that created it, so `m[k]=new()` followed by `m[k].push()` read back
//! `n=0` and `m.exists(k)` was false — the config-db waiters never woke.
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/static_instance_assoc.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/static_instance_assoc.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"class wq;
  int n;
  function new(); n = 0; endfunction
  function void push(); n++; endfunction
endclass

class db_impl #(type T = int);
  static local wq m_waiters[string];

  // An instance (non-static) method registering/mutating the shared static
  // assoc cell — UVM wait_modified's shape.
  task wait_modified(string field);
    if (!m_waiters.exists(field))
      m_waiters[field] = new();
    m_waiters[field].push();
  endtask

  static function int queued(string field);
    return m_waiters.exists(field) ? m_waiters[field].n : -1;
  endfunction
endclass

module top;
  initial begin
    db_impl #(bit) a = new;
    db_impl #(bit) b = new;
    // Two watcher processes (instance methods) touching the SAME field.
    fork
      a.wait_modified("field1");
      b.wait_modified("field1");
    join_none
    #5;
    if (db_impl #(bit)::queued("field1") == 2)
      $display("TAG_PASS queued=%0d", db_impl #(bit)::queued("field1"));
    else
      $display("TAG_FAIL queued=%0d", db_impl #(bit)::queued("field1"));
    $finish;
  end
endmodule
"#;

#[test]
fn static_instance_method_assoc_object_shared() {
    let out = run(SRC);
    assert!(
        out.contains("TAG_PASS queued=2"),
        "two instance-method register() calls on one parameterized static assoc \
         field must share the object cell (n=2), got:\n{out}"
    );
    assert!(
        !out.contains("TAG_FAIL"),
        "unexpected instance-method static-assoc failure:\n{out}"
    );
}
