//! IEEE 1800-2017 §13.5.2 / §26.3: same-named package functions
//! retain their own argument directions during ref-actual validation.
use xezim::simulate;

#[test]
fn package_input_signature_is_not_replaced_by_a_ref_signature() {
    let sim = simulate(
        r#"package a_by_ref;
  function automatic void route(ref int value); value++; endfunction
endpackage
package b_by_value;
  function automatic void route(input int value);
    if (value != 7) $fatal(1, "wrong input value");
  endfunction
  function automatic void run(); route(7); endfunction
endpackage
module top;
  initial begin
    b_by_value::run();
    $display("PASS: ref validation uses the callee's package");
    $finish;
  end
endmodule"#,
        10,
    )
    .expect("a literal is valid for the input formal");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["PASS: ref validation uses the callee's package"]);
}

#[test]
fn package_tasks_and_qualified_ref_calls_keep_their_signatures() {
    let sim = simulate(
        r#"package a_by_ref;
  function automatic void route(ref int value); value++; endfunction
  task automatic send(ref int value); value++; endtask
endpackage
package b_by_value;
  function automatic void route(input int value);
    if (value != 7) $fatal(1, "wrong function input");
  endfunction
  task automatic send(input int value);
    if (value != 9) $fatal(1, "wrong task input");
  endtask
  task automatic run(); route(.value(7)); send(9); endtask
endpackage
module top;
  initial begin
    int value = 1;
    b_by_value::run();
    a_by_ref::route(value);
    a_by_ref::send(value);
    $display("ref value=%0d", value);
    $finish;
  end
endmodule"#,
        10,
    )
    .expect("each package retains its signature and ref writeback");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["ref value=3"]);
}
