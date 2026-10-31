//! IEEE 1800-2017 §8.24 / §26.3: an out-of-block constructor or method
//! belongs to its class and is not a package-level wildcard-import candidate.
use xezim::simulate;

#[test]
fn wildcard_imports_do_not_make_out_of_block_constructors_ambiguous() {
    let sim = simulate(
        r#"package first_pkg;
  class first_class;
    int value;
    extern function new(int initial_value);
    extern task route();
    extern function int sample();
  endclass
  function first_class::new(int initial_value); value = initial_value; endfunction
  task first_class::route(); value++; endtask
  function int first_class::sample(); return value; endfunction
endpackage
package second_pkg;
  class second_class;
    int value;
    extern function new(int initial_value);
  endclass
  function second_class::new(int initial_value); value = initial_value; endfunction
  function automatic int route(); return 99; endfunction
  function automatic int sample(); return 42; endfunction
endpackage
module top;
  import first_pkg::*;
  import second_pkg::*;
  first_pkg::first_class a;
  second_pkg::second_class b;
  initial begin
    a = new(10); b = new(20);
    a.route();
    $display("constructors %0d %0d free %0d %0d", a.sample(), b.value, route(), sample());
    $finish;
  end
endmodule"#,
        10,
    )
    .expect("constructors and out-of-block methods retain their class scope");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["constructors 11 20 free 99 42"]);
}

#[test]
fn genuine_package_task_collisions_are_still_ambiguous() {
    let err = match simulate(
        r#"package first_pkg; task automatic route(); endtask endpackage
package second_pkg; task automatic route(); endtask endpackage
module top;
  import first_pkg::*; import second_pkg::*;
  initial route();
endmodule"#,
        10,
    ) {
        Ok(_) => panic!("accepted an ambiguous package task"),
        Err(err) => err,
    };
    assert!(
        err.contains("more than one wildcard-imported package"),
        "{err}"
    );
}
