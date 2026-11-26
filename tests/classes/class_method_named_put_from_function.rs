//! A user-defined VOID method named `put` called as a STATEMENT from
//! inside a function body was silently swallowed by the expression-context
//! mailbox/semaphore `put` interception (`eval_call_inner`'s `mname ==
//! "put"` arm returned `Value::zero(32)` for a non-container receiver
//! without ever consulting the class method table). The same call made
//! directly from an `initial`/`always` process worked — the process
//! statement executor resolves the receiver+method through the class task
//! table before the builtin arms run — which made this look like a
//! call-depth bug rather than a name-collision one.
//!
//! The consequence inside UVM was severe: `uvm_re_match` (a package
//! function) does `cache.put(re, rexp)` on a `uvm_regex_cache`, so the
//! regex cache NEVER hit and every `uvm_is_match` recompiled its regex
//! through the DPI (uvm_regex_cache::put → uvm_lru_cache::put).
//!
//! Fix: mirror the `get` arm's `is_user_get` guard — when the receiver is
//! a live class instance whose class defines a user `put`, fall through
//! to the generic method dispatch instead of returning zero.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// The minimal repro: `c.put("a", 1)` inside a module-level function
/// must execute the class method body (assoc-array member write
/// observable through a later `get`).
#[test]
fn class_method_put_from_function_context() {
    const SRC: &str = "module tb;
  class cache;
    int m_hash[string];
    int m_mark;
    function void put(string k, int v);
      m_mark = v;
      m_hash[k] = v;
    endfunction
    function int get(string k);
      return m_hash.exists(k) ? m_hash[k] : -1;
    endfunction
  endclass
  cache c;
  int direct_, wrapped, mark;
  function void wrapper();
    c.put(\"a\", 1);
  endfunction
  initial begin
    c = new();
    c.put(\"x\", 5);   // direct call from a process — always worked
    wrapper();        // nested call — body was silently dropped
    direct_ = c.get(\"x\");
    wrapped = c.get(\"a\");
    mark = c.m_mark;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "direct_"), 5, "direct put still works");
    assert_eq!(
        u(&sim, "wrapped"),
        1,
        "put inside a function must execute the class method"
    );
    assert_eq!(
        u(&sim, "mark"),
        1,
        "the nested put's body must run in order (m_mark=1)"
    );
}

/// §13.4, §15.4: a function can use the nonblocking mailbox operation.
#[test]
fn real_mailbox_try_put_from_function_delivers() {
    const SRC: &str = "module tb;
  mailbox mb;
  int got;
  function void produce();
    void'(mb.try_put(42));
  endfunction
  initial begin
    mb = new();
    produce();
    mb.get(got);
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "got"),
        42,
        "mailbox try_put from a function delivers"
    );
}

/// Same for a semaphore `put` from a function context.
#[test]
fn real_semaphore_put_from_function_still_counts() {
    const SRC: &str = "module tb;
  semaphore sem;
  int taken;
  function void give();
    sem.put(2);
  endfunction
  initial begin
    sem = new();
    give();
    if (sem.try_get(2)) taken = 1;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "taken"),
        1,
        "semaphore put from a function still increments"
    );
}
