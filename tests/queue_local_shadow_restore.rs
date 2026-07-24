//! IEEE 1800-2020 §6.21 — automatic (per-invocation) local storage, and
//! §7.4 — queue `delete()`, in the presence of bare-name collision across
//! call frames inside a CLASS method.
//!
//! A local `string p[$]` in a class method `check()` is keyed by its bare
//! name in the simulator's global queue tables. A nested call that declares
//! its OWN local of the same name (here `helper`'s `int p`) legitimately
//! shadows it for the duration of the nested call — but when the nested
//! call returns, the caller's `p` queue registration MUST be restored, or
//! the caller's subsequent `p.delete()` is a silent no-op.
//!
//! Without the fix, `p.size()` kept the stale pre-call count (the nested
//! helper's same-named local unregistered the caller's queue, so `delete()`
//! fell through every guard and did nothing) and `p[i]` reads returned
//! empty. After the fix, `delete()` clears and the two later pushes bring
//! the size back to 2.
//!
//! This was the real blocker for UVM `09callbacks/20inherit`: its
//! `check_phase` declares `string p[$]` and rebuilds it across 6 blocks
//! with `p.delete()` between them; UVM-internal helpers reached during
//! `uvm_callbacks#(...)::display()` declared their own `p`, unregistering
//! the caller's queue. The test then saw `p.size()` grow unbounded with
//! every read empty, so every component reported failure regardless of the
//! actual callback queues.
//!
//! Verified byte-for-byte against reference simulators (OUT_SIZE == 2).

use xezim::simulate;

const SRC: &str = r#"
module top;
  class c;
    function int helper();
      int p;
      p = 99;
      return p;
    endfunction
    function int check();
      string p[$];
      p.push_back("first");
      p.push_back("second");
      void'(this.helper());  // nested call shadows & must restore `p`
      p.delete();            // no-op without the fix (registration lost)
      p.push_back("third");
      p.push_back("fourth");
      return p.size();       // 2 with the fix, 4 without
    endfunction
  endclass
  c obj;
  int out_size;
  initial begin
    obj = new;
    out_size = obj.check();
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(&format!("top.{}", n))
        .or_else(|| sim.get_signal(n))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or(0)
}

#[test]
fn queue_local_restored_after_nested_same_name_shadow() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // `delete()` must clear `p` despite the nested helper's same-named local
    // unregistering it; the two later pushes bring size back to 2 (was 4
    // before the fix — delete silently no-op'd, stale count kept growing).
    assert_eq!(
        u(&sim, "out_size"),
        2,
        "p.size() after delete+2 pushes must be 2 (delete must clear)"
    );
}
