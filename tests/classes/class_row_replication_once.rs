//! IEEE 1800-2023 §11.4.12.1, §10.9.1: the operands of a replication in an
//! assignment pattern are evaluated exactly once, however many elements the
//! copies fill. A row of a class property (`k.a[0] = '{2{f()}}`, a row of a
//! fixed array or of a queue or dynamic array of rows) is stored by the class
//! row path, which evaluated the replicated item once for each element, so
//! `f` ran twice and the row got two different values. The one evaluation is
//! still in the row's element type: a `byte` operand plus `8'd1` stored into a
//! `longint` row keeps its carry (§11.6.1).

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn replicated_call_runs_once_into_a_class_row() {
    let out = lines(
        r#"
module top;
  int calls;
  function automatic int f(); calls++; return calls * 10; endfunction
  function automatic byte unsigned g(); calls++; return 8'd255; endfunction
  class K;
    longint w [2][2];
    int a [2][2];
    int q [$][2];
    int d [][3];
    function void fill(); a[1] = '{2{f()}}; endfunction
  endclass
  K k;
  initial begin
    k = new;
    calls = 0; k.a[0] = '{2{f()}};
    $display("T|a calls=%0d a[0]=%0d %0d", calls, k.a[0][0], k.a[0][1]);
    k.q.push_back('{0, 0});
    calls = 0; k.q[0] = '{2{f()}};
    $display("T|q calls=%0d q[0]=%0d %0d", calls, k.q[0][0], k.q[0][1]);
    k.d = new[1];
    calls = 0; k.d[0] = '{3{f()}};
    $display("T|d calls=%0d d[0]=%0d %0d %0d", calls, k.d[0][0], k.d[0][1], k.d[0][2]);
    calls = 0; k.fill();
    $display("T|m calls=%0d a[1]=%0d %0d", calls, k.a[1][0], k.a[1][1]);
    calls = 0; k.a[1] = '{f(), f()};
    $display("T|two calls=%0d a[1]=%0d %0d", calls, k.a[1][0], k.a[1][1]);
    calls = 0; k.w[0] = '{2{g() + 8'd1}};
    $display("T|w calls=%0d w[0]=%0d %0d", calls, k.w[0][0], k.w[0][1]);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|a calls=1 a[0]=10 10",
            "T|q calls=1 q[0]=10 10",
            "T|d calls=1 d[0]=10 10 10",
            "T|m calls=1 a[1]=10 10",
            "T|two calls=2 a[1]=10 20",
            "T|w calls=1 w[0]=256 256",
        ]
    );
}
