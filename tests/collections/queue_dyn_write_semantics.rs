//! §7.5.1/§7.6/§7.10.1 — queue write-at-size appends, dynamic array resizes
//! on assignment from a fixed array, and 4-state new[] elements init to x.
//! Reference-validated (audit round I12/I13/I15).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn queue_write_at_size_appends() {
    let src = r#"
module tb;
  int q[$];
  int sz, q0;
  initial begin
    q[0] = 5;
    sz = q.size(); q0 = q[0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 1, "q[0]=v on empty queue appends");
    assert_eq!(u(&sim, "q0"), 5);
}

#[test]
fn dynamic_array_resizes_on_fixed_assign() {
    let src = r#"
module tb;
  int d[];
  int f[3];
  int sz, d2;
  initial begin
    d = new[4];
    f = '{7, 8, 9};
    d = f;
    sz = d.size(); d2 = d[2];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 3, "assignment resizes to the source size");
    assert_eq!(u(&sim, "d2"), 9);
}

#[test]
fn four_state_new_elements_init_x() {
    let src = r#"
module tb;
  logic [3:0] d[];
  bit [3:0] b[];
  int d0_is_x, b0;
  initial begin
    d = new[2];
    b = new[2];
    d0_is_x = (d[0] === 4'bxxxx);
    b0 = b[0]; // 2-state stays 0
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "d0_is_x"), 1, "4-state new[] elements are x");
    assert_eq!(u(&sim, "b0"), 0, "2-state new[] elements are 0");
}
