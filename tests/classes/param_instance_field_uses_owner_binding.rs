//! A parameterized class's TYPE-PARAMETER-typed instance field must resolve
//! to the concrete binding of the OWNING instance's class chain — not to a
//! sibling specialization's active `current_spec`.
//!
//! `seq_p #(REQ)` declares `REQ req`. `top_seq extends seq_p #(item_a)` and
//! `bot_seq extends seq_p #(item_b)` are structurally identical but distinct
//! bindings. Dispatching the static `seq_p` method `probe_top_req_type` through
//! the concrete receiver `bot_seq` seeds xezim's active specialization with
//! `(seq_p, item_b)` for that body (exactly the leakage a concurrently running
//! UVM sequencer of the other item type produced). While that foreign
//! specialization is active, resolving `t.req` for a `top_seq` must consult the
//! owner's own class chain and yield `item_a`, NOT the contaminated `item_b`.
//!
//! Pre-fix xezim preferred the global `current_spec` over the per-instance
//! ancestor binding, so `t.req.get_type()` returned `item_b` (wrong) — the root
//! of UVM's `SQRSNDREQCAST` send_request cast failure when `top_sequence` and
//! `bot_sequence` ran concurrently.
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
    std::fs::write("/tmp/param_instance_field_uses_owner_binding.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/param_instance_field_uses_owner_binding.sv"])
        .output()
        .expect("run xezim");
    let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
    all.push_str(&String::from_utf8_lossy(&out.stderr));
    all
}

const SRC: &str = r#"module top;
  typedef class top_seq;
  typedef class bot_seq;

  class item_a;
    static function string get_type(); return "item_a"; endfunction
  endclass
  class item_b;
    static function string get_type(); return "item_b"; endfunction
  endclass

  virtual class seq_p #(type REQ);
    REQ req;
    // Static method declared in the PARAMETERIZED base; reached through the
    // concrete non-parameterized receiver `bot_seq` so the body runs with the
    // bot specialization active. It inspects a top_seq's `req` FIELD directly
    // (no instance-method call that would re-derive top's own spec).
    static function string probe_top_req_type(top_seq t);
      string s = t.req.get_type();
      return s;
    endfunction
  endclass

  class top_seq extends seq_p #(item_a); endclass
  class bot_seq extends seq_p #(item_b); endclass

  string n;
  top_seq t;
  initial begin
    t = new;
    n = bot_seq::probe_top_req_type(t);
    $display("TAG_REQ_TYPE=%s", n);
    if (n == "item_a") $display("TAG_PASS");
    else begin
      $display("TAG_FAIL: resolved %s instead of item_a", n);
    end
    $finish;
  end
endmodule
"#;

#[test]
fn param_instance_typeparam_field_uses_owner_binding() {
    let out = run(SRC);
    assert!(
        out.contains("TAG_REQ_TYPE=item_a"),
        "owner's own class chain must win over the contaminated current_spec; got:\n{out}"
    );
    assert!(
        out.contains("TAG_PASS"),
        "buggy resolution (item_b) must be rejected:\n{out}"
    );
}