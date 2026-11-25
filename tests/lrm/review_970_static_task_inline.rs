//! #970 review: §6.21, §13.3.1: a variable of a static task keeps its value
//! between calls, which a compiled inline with fresh registers cannot do. That
//! only matters for a variable whose value can carry over: a scratch local or
//! an output formal written whole before it is read behaves the same either
//! way, so such a task still inlines on the compiled tier, while a local read
//! before it is written keeps its static value. Expected values come from the
//! reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// §6.21, §8.6, §13.4.2: automatic declarations belong to the callee.
#[test]
fn static_task_preserves_automatic_callee_initializers() {
    let src = r#"
module top;
  class C;
    function int method_value(int a);
      int local_value = a + 2;
      return local_value;
    endfunction
  endclass
  C c;
  function automatic int function_value(int a);
    int local_value = a + 1;
    return local_value;
  endfunction
  task static run_checks(int a);
    int function_result, method_result;
    function_result = function_value(a);
    method_result = c.method_value(a);
    begin
      static int calls = 0;
      calls++;
      $display("values=%0d,%0d calls=%0d", function_result, method_result, calls);
    end
  endtask
  initial begin
    c = new;
    run_checks(10);
    run_checks(20);
    run_checks(30);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "values=11,12 calls=1",
            "values=21,22 calls=2",
            "values=31,32 calls=3"
        ]
    );
}

#[test]
fn written_first_locals_inline_and_carried_locals_stay_static() {
    let src = r#"module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] st [4];
  int a = 0, b = 0, c = 0;
  task mix(output int o);
    integer k;
    o = 0;
    for (k = 0; k < 4; k = k + 1) begin
      st[k] = st[k] + 8'(k + 1);
      o = o + st[k];
    end
  endtask
  task carry();
    int w;
    w++;
    b += w;
  endtask
  always @(posedge clk) begin mix(a); carry(); c++; end
  initial begin
    foreach (st[i]) st[i] = 8'(i * 16);
    #100 $display("a=%0d b=%0d c=%0d st=%0d %0d %0d %0d", a, b, c, st[0], st[1], st[2], st[3]);
    $finish;
  end
endmodule"#;
    assert_eq!(lines(src), ["a=196 b=55 c=10 st=10 36 62 88"]);
}
