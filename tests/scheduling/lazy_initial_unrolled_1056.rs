//! Test deferred initial block materialization (issue #1056).

use xezim::simulate;

#[test]
fn unrolled_initial_materialized_at_dispatch_1056() {
    let mut body = String::new();
    for i in 0..128 {
        body.push_str(&format!("    acc = acc + {};\n", i));
    }
    let src = format!(
        r#"
module element #(parameter int KEY = 0)();
  int acc;
  initial begin
    acc = 0;
{}    if (acc != 8128) $fatal(1, "sum");
  end
endmodule

module top;
  for (genvar j = 0; j < 64; j = j + 1) begin: g
    element #(.KEY(j)) u();
  end
  initial begin
    #1;
    $display("PASS acc0=%0d acc63=%0d", g[0].u.acc, g[63].u.acc);
    $finish;
  end
endmodule
"#,
        body
    );

    let res = simulate(&src, 1000).expect("simulation should succeed");
    let messages: Vec<_> = res.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("PASS acc0=8128 acc63=8128")),
        "unexpected output: {messages:?}"
    );
}
