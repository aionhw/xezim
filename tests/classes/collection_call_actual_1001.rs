//! §13.5.1, §7.6: collection-valued actuals copy a call's whole return value,
//! in caller scope, exactly once. Includes the original issue's 18 probes.

#[test]
fn reported_collection_call_actuals_preserve_values() {
    let sim = xezim::simulate(
        include_str!("../issue_cases/collection_call_actual_1001.sv"),
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"TEST_PASS"), "{lines:?}");
    assert!(!lines.iter().any(|s| s.contains("FAIL:")), "{lines:?}");
}

#[test]
fn call_actuals_are_copied_before_the_next_actual_runs() {
    let sim = xezim::simulate(
        r#"
typedef int iq_t[$];
module top;
  int calls;
  function automatic iq_t make(int n);
    calls++;
    return '{n,n+1};
  endfunction
  function automatic int use_values(iq_t a, iq_t b);
    a[0]++;
    return a[0]*100+b[0];
  endfunction
  initial begin
    int result;
    result=use_values(make(10),make(20));
    $display("T|calls=%0d result=%0d", calls, result);
    if (calls!=2 || result!=1120) $fatal(1,"actual evaluated twice or overwritten");
    result=use_values((1 ? make(30) : make(40)),make(50));
    if (calls!=4 || result!=3150) $fatal(1,"selected actual evaluated twice");
    $display("T|PASS");
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}
