//! §8.7 constructor-time method binding and §21.2.1.7 %p of a class
//! fixed-array property. Reference-validated (audit round I18/I19).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn base_ctor_binds_own_method() {
    let src = r#"
class B;
  int r;
  virtual function int f(); return 1; endfunction
  function new(); r = f(); endfunction
endclass
class D extends B;
  int y = 5;
  virtual function int f(); return y + 100; endfunction
endclass
module tb;
  int dr;
  initial begin
    D d = new;
    dr = d.r;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "dr"), 1, "B::new binds B::f, not the derived override");
}

#[test]
fn percent_p_class_fixed_array_property() {
    let src = r#"
class C;
  int arr[3];
endclass
module tb;
  initial begin
    C c = new;
    c.arr[1] = 9;
    $display("T|p=%p", c.arr);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with("T|p="))
        .expect("output line");
    assert_eq!(line, "T|p='{0, 9, 0}");
}
