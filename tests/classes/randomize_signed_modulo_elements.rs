//! `%` on signed array elements in a `foreach` constraint (IEEE 1800-2023
//! §18.5.8.1, §18.6.1, §18.12). The solver leaves a signed `%` to the
//! evaluator, which judges `a[i] % 16 == 0` once `a[i]` is fixed. It used
//! to wait for every element of `a`, so `randomize()` gave up on
//! satisfiable sets once the array had more than a few elements.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// `std::randomize` of a local array of 12 signed elements, each a
/// multiple of 16.
#[test]
fn std_randomize_signed_elements_modulo() {
    const SRC: &str = "module tb;
  int fails = 0;
  int bad = 0;
  initial begin
    repeat (20) begin
      int a[12];
      if (!std::randomize(a) with { foreach (a[i]) { a[i] inside {[-2048:0]}; a[i] % 16 == 0; } })
        fails++;
      else foreach (a[i]) if (a[i] < -2048 || a[i] > 0 || a[i] % 16 != 0) bad++;
    end
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "fails"),
        0,
        "every std::randomize call must succeed"
    );
    assert_eq!(u(&sim, "bad"), 0, "every element must meet its constraints");
}

/// A walk from 0 back to 0 in `n` signed steps, each a nonzero multiple of
/// 16, through a class and through `std::randomize`. Every `n` in 2..12
/// has solutions, so the calls succeed and long walks occur.
#[test]
fn signed_step_modulo_walk() {
    const SRC: &str = "`define WALK \\
  n inside {[2:12]}; \\
  a.size() == n + 1; \\
  a[0] == 0; \\
  a[n] == 0; \\
  foreach (a[i]) { \\
    a[i] inside {[-2048:0]}; \\
    if (i > 0) { \\
      (a[i] - a[i-1]) % 16 == 0; \\
      (a[i] - a[i-1]) inside {[-112:-16], [16:112]}; \\
    } \\
  }

class walk;
  rand int n;
  rand int a[];
  constraint k { `WALK }
endclass

module tb;
  int n;
  int a[];
  int fails = 0;
  int bad = 0;
  int longest = 0;

  function automatic void check(int n, int a[]);
    if (n > longest) longest = n;
    if (a.size() != n + 1 || a[0] != 0 || a[n] != 0) bad++;
    for (int i = 1; i < a.size(); i++)
      if ((a[i] - a[i-1]) % 16 != 0 || !((a[i] - a[i-1]) inside {[-112:-16], [16:112]}))
        bad++;
  endfunction

  initial begin
    walk w = new;
    repeat (30) begin
      if (!std::randomize(a, n) with { `WALK }) fails++;
      else check(n, a);
      if (!w.randomize()) fails++;
      else check(w.n, w.a);
    end
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "fails"), 0, "every randomize call must succeed");
    assert_eq!(u(&sim, "bad"), 0, "every walk must meet its constraints");
    assert!(
        u(&sim, "longest") >= 8,
        "walks of 8 or more steps must occur"
    );
}
