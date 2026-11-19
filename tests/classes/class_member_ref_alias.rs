//! IEEE 1800-2023 §13.5.2: writes through a `ref` formal are visible
//! through the actual before the subroutine returns.

use xezim::simulate;

fn check(src: &str, expected: &str) {
    let sim = simulate(src, 10).expect("simulate failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, [expected]);
}

#[test]
fn method_property_and_task_array_element_update_during_call() {
    let src = r#"
module top;
  class C;
    int value = 6;
    int arr[2] = '{1, 2};
    int seen;
    function void bump(ref int dst);
      dst += 5;
      seen = value;
    endfunction
    function void run();
      bump(value);
    endfunction
  endclass
  C c;
  int seen_element;
  task automatic add(ref int x);
    x += 5;
    seen_element = c.arr[1];
  endtask
  initial begin
    c = new;
    c.run();
    add(c.arr[1]);
    $display("T|property %0d seen %0d|element %0d seen %0d", c.value, c.seen, c.arr[1], seen_element);
  end
endmodule
"#;
    check(src, "T|property 11 seen 11|element 7 seen 7");
}

#[test]
fn method_queue_element_updates_during_call() {
    let src = r#"
module top;
  class C;
    int q[$] = '{5, 7};
    int seen;
    function void bump(ref int dst);
      dst += 3;
      seen = q[1];
    endfunction
    function void run();
      bump(q[1]);
    endfunction
  endclass
  C c;
  initial begin
    c = new;
    c.run();
    $display("T|queue element %0d seen %0d", c.q[1], c.seen);
  end
endmodule
"#;
    check(src, "T|queue element 10 seen 10");
}

#[test]
fn task_class_property_updates_during_call() {
    let src = r#"
module top;
  class C;
    int v = 6;
  endclass
  C c;
  task automatic bump(ref int x);
    x += 5;
    $display("T|property %0d", c.v);
  endtask
  initial begin
    c = new;
    bump(c.v);
  end
endmodule
"#;
    check(src, "T|property 11");
}

#[test]
fn task_module_variable_updates_during_call() {
    let src = r#"
module top;
  int m = 6;
  task automatic bump(ref int x);
    x += 5;
    $display("T|module variable %0d", m);
  endtask
  initial bump(m);
endmodule
"#;
    check(src, "T|module variable 11");
}
