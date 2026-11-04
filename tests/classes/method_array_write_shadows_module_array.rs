//! IEEE 1800-2023 §8.3 / §23.9: inside a class method, a bare array name
//! that the class declares is the object's member. Its element write
//! `a[i] = v` (a `foreach` fill) found a same-named module-level array's
//! element in the signal table first, so the module array took the values
//! and the object's member stayed 0, while the read side already resolved
//! to the member (sibling of #519 / #534: the name-keyed store outranked
//! the member lift on the write side only).

use xezim::simulate;

#[test]
fn method_element_write_targets_the_member_not_a_module_array() {
    let src = r#"
class D;
  int a [4];
  function void seed(); foreach (a[i]) a[i] = 7 + i; endfunction
  function int total(); int s = 0; foreach (a[i]) s += a[i]; return s; endfunction
endclass
module tb;
  int a [4];
  D d;
  initial begin
    a[0] = 1; a[3] = 3;
    d = new; d.seed();
    $display("module a0=%0d a3=%0d object a0=%0d a3=%0d total=%0d",
             a[0], a[3], d.a[0], d.a[3], d.total());
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let out: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        out.iter().any(|m| m.contains("module a0=1 a3=3 object a0=7 a3=10 total=34")),
        "{out:?}"
    );
}
