//! IEEE 1800-2017 Clause 13 conformance findings — tasks and functions.
//!
//! §13.5.4 named arguments (`f(.b(2), .a(1))`) were IGNORED. Argument binding
//! is positional everywhere, so a `.name(expr)` actual was evaluated as an
//! opaque expression in whatever slot it happened to occupy: it bound the wrong
//! formal, or nothing at all. `f_named(.a(1), .b(2))` returned 0, and a task's
//! `.o(result)` output argument silently never wrote back.
//!
//! §13.5.2 unpacked ARRAY arguments. Tasks bound only the `[lo:hi]` range
//! spelling; functions bound none at all. So `int a[3]` — the common form —
//! passed nothing: the body read X and a `ref` write never reached the caller.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  // --- named / default arguments -------------------------------------------
  task automatic t_pos (input int a, input int b, output int o); o = a + b; endtask
  task automatic t_dflt(input int a = 5, input int b = 6, output int o); o = a + b; endtask
  function automatic int f_named(int a, int b);        return a*10 + b; endfunction
  function automatic int f_dflt (int a = 2, int b = 3); return a*10 + b; endfunction

  // --- array arguments -----------------------------------------------------
  function automatic void f_bump(ref int arr[3]);            arr[1] = 77; endfunction
  function automatic int  f_sum (const ref int arr[3]);      return arr[0]+arr[1]+arr[2]; endfunction
  task     automatic void_t     (ref int arr[3]);            arr[2] = 55; endtask
  function automatic int  f_in  (int arr[3]);                return arr[0]+arr[2]; endfunction
  task     automatic t_range    (ref int arr[2:0]);          arr[0] = 11; endtask

  int r_pos, r_named, r_dflt_named, r_dflt_pos;
  int fn_named, fn_dflt0, fn_dflt1, fn_dflt_named;
  int a[3], b[3], c[2:0];
  int bumped, summed, task_wrote, in_only, range_wrote;

  initial begin
    r_pos = 0;        t_pos(1, 2, r_pos);
    r_named = 0;      t_pos(.a(1), .b(2), .o(r_named));
    r_dflt_named = 0; t_dflt(.o(r_dflt_named));          // both inputs defaulted
    r_dflt_pos = 0;   t_dflt(1, 2, r_dflt_pos);

    fn_named      = f_named(.a(1), .b(2));
    fn_dflt0      = f_dflt();
    fn_dflt1      = f_dflt(9);
    fn_dflt_named = f_dflt(.b(9));

    a = '{1,2,3}; f_bump(a);   bumped = a[1];
    b = '{1,2,3}; summed = f_sum(b);
    a = '{1,2,3}; void_t(a);   task_wrote = a[2];
    b = '{4,5,6}; in_only = f_in(b);
    // An `input` array must NOT be written back.
    b = '{1,2,3};
    c = '{0,0,0}; t_range(c); range_wrote = c[0];
  end
endmodule
"#;

/// An `input` array formal must not leak writes back to the caller.
const INPUT_ARRAY: &str = r#"
module tb;
  function automatic int clobber(int arr[3]);
    arr[0] = 999;
    return arr[0];
  endfunction
  int a[3];
  int ret, kept;
  initial begin
    a = '{1,2,3};
    ret  = clobber(a);
    kept = a[0];
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
fn named_arguments_bind_to_the_right_formal() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r_pos"), 3);
    assert_eq!(u(&sim, "r_named"), 3, "named task args were ignored");
    assert_eq!(u(&sim, "fn_named"), 12, "named function args were ignored");
}

#[test]
fn omitted_formals_take_their_defaults_with_named_args() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // t_dflt(.o(r)) -> 5 + 6
    assert_eq!(u(&sim, "r_dflt_named"), 11);
    assert_eq!(u(&sim, "r_dflt_pos"), 3);
    // f_dflt defaults are a=2, b=3
    assert_eq!(u(&sim, "fn_dflt0"), 23);
    assert_eq!(u(&sim, "fn_dflt1"), 93);
    assert_eq!(u(&sim, "fn_dflt_named"), 29, "f_dflt(.b(9)) must keep a's default");
}

#[test]
fn unpacked_array_arguments_pass_in_and_ref_writes_come_back() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // `ref int arr[3]` in a FUNCTION — functions bound no arrays at all.
    assert_eq!(u(&sim, "bumped"), 77);
    // `const ref` reads the caller's contents rather than X.
    assert_eq!(u(&sim, "summed"), 6);
    // ...and in a task, for both the size and the range spelling.
    assert_eq!(u(&sim, "task_wrote"), 55);
    assert_eq!(u(&sim, "range_wrote"), 11);
    // A plain `input` array is readable.
    assert_eq!(u(&sim, "in_only"), 10);
}

#[test]
fn an_input_array_formal_is_not_written_back() {
    let sim = simulate(INPUT_ARRAY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ret"), 999, "the formal itself is writable");
    assert_eq!(u(&sim, "kept"), 1, "an input array must not write back to the caller");
}
