//! Three defects found by an IEEE 1800-2017 associative-array compliance
//! testbench. Only one of them is actually an associative-array bug.
//!
//! 1. A PACKED STRUCT declared inside a task lived in the call frame, but the
//!    member-slice paths only ever looked in the signal table. So `s.id = 8'hAA`
//!    reached nothing and `s` read back as X. When such a struct is used as an
//!    associative-array key, every key is X and they all collide — which is how
//!    this surfaced. The same declaration in an `initial` block worked, because
//!    there the local becomes a signal.
//!
//! 2. §13.5.2 — a function never bound an ASSOCIATIVE-array formal (tasks did),
//!    so `arr.size()` inside the body read 0.
//!
//! 3. §7.8.2 — an associative index containing x/z is invalid. The unknown bits
//!    were folded away, so `a[4'b1x01] = 55` silently clobbered `a[4'b1001]`.
//!    The access is now ignored, with a warning, and a read returns the default.

use xezim::simulate;

const PACKED_LOCAL: &str = r#"
module tb;
  typedef struct packed { logic [7:0] id; logic valid; } k_t;
  k_t mod_s;
  int mod_v, init_v, task_v;
  int key_a, key_b, key_n;

  task automatic t();
    k_t task_s;
    int m[k_t];
    k_t s1, s2;
    task_s.id = 8'hCC; task_s.valid = 1'b1;
    task_v = task_s;

    // A packed struct as an associative key.
    s1.id = 8'hAA; s1.valid = 1'b1;
    s2.id = 8'hBB; s2.valid = 1'b0;
    m[s1] = 100;
    m[s2] = 200;
    key_a = m[s1];
    key_b = m[s2];
    key_n = m.num();
  endtask

  initial begin
    k_t init_s;
    mod_s.id  = 8'hAA; mod_s.valid = 1'b1;
    init_s.id = 8'hBB; init_s.valid = 1'b0;
    mod_v  = mod_s;
    init_v = init_s;
    t();
  end
endmodule
"#;

const ASSOC_ARG: &str = r#"
module tb;
  function automatic int f_ref(ref int arr[int]);   return arr.size(); endfunction
  function automatic int f_val(int arr[int]);       return arr.size(); endfunction
  task     automatic t_ref(ref int arr[int], output int n); n = arr.size(); endtask

  int a[int];
  int fn_ref, fn_val, tk_ref, after;
  initial begin
    a[1] = 10; a[2] = 20;
    fn_ref = f_ref(a);
    fn_val = f_val(a);
    t_ref(a, tk_ref);
    after = a.size();     // the caller's array survives
  end
endmodule
"#;

const XZ_INDEX: &str = r#"
module tb;
  int a[logic [3:0]];
  logic [3:0] good = 4'b1001;
  logic [3:0] bad  = 4'b1x01;
  int good_v, bad_v, n;
  initial begin
    a[good] = 11;
    a[bad]  = 55;      // invalid index: ignored
    good_v = a[good];  // must NOT have been clobbered
    bad_v  = a[bad];   // must read the default
    n = a.num();
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn a_packed_struct_declared_in_a_task_aliases_its_members() {
    let sim = simulate(PACKED_LOCAL, 100).expect("simulate failed");
    // 8'hAA with valid=1 -> 9'h155
    assert_eq!(u(&sim, "mod_v"), 0x155, "module scope regressed");
    assert_eq!(u(&sim, "init_v"), 0x176, "initial-block local regressed");
    assert_eq!(u(&sim, "task_v"), 0x199, "a task-local packed struct read back X");
}

#[test]
fn a_packed_struct_works_as_an_associative_key() {
    let sim = simulate(PACKED_LOCAL, 100).expect("simulate failed");
    assert_eq!(u(&sim, "key_a"), 100);
    assert_eq!(u(&sim, "key_b"), 200);
    assert_eq!(u(&sim, "key_n"), 2, "both struct keys collided");
}

#[test]
fn associative_arrays_bind_to_function_arguments() {
    let sim = simulate(ASSOC_ARG, 100).expect("simulate failed");
    assert_eq!(u(&sim, "fn_ref"), 2, "a function never bound an assoc formal");
    assert_eq!(u(&sim, "fn_val"), 2);
    assert_eq!(u(&sim, "tk_ref"), 2);
    assert_eq!(u(&sim, "after"), 2, "the caller's array was disturbed");
}

#[test]
fn an_unknown_associative_index_is_ignored() {
    let sim = simulate(XZ_INDEX, 100).expect("simulate failed");
    assert_eq!(u(&sim, "good_v"), 11, "an x/z index clobbered a real element");
    assert_eq!(u(&sim, "bad_v"), 0, "reading with an x/z index must yield the default");
    assert_eq!(u(&sim, "n"), 1, "an x/z index created an element");
}
