//! IEEE 1800-2023 §12.7.3: an omitted `foreach` loop variable skips that
//! dimension. Before a queue, dynamic or associative dimension the
//! element's size depends on the skipped index, so the loop is rejected.
//! It used to pad the missing dimension with the element's packed width and
//! walk the `int` row's 32 bits (#536, reported by chenbhil).

use xezim::simulate;

fn rejects(src: &str) {
    let sim = simulate(src, 10).expect("simulate");
    assert!(sim.saw_fatal, "expected the run to abort");
    assert!(
        sim.output.iter().any(|o| o.message.contains("cannot be omitted")),
        "{:?}",
        sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn omitted_variable_before_a_queue_dimension_is_rejected() {
    rejects(r#"
module tb;
  int m [3][3][$];
  initial begin
    int n;
    m[2][0].push_back(1); m[2][0].push_back(2); m[0][1].push_back(3);
    n = 0; foreach (m[i, , k]) n++;
    $display("n=%0d", n);
  end
endmodule
"#);
}

#[test]
fn omitted_variable_before_a_dynamic_dimension_is_rejected() {
    rejects(r#"
module tb;
  int md [2][2][];
  initial begin
    int n;
    md[1][0] = new[3];
    n = 0; foreach (md[i, , k]) n++;
    $display("n=%0d", n);
  end
endmodule
"#);
}

#[test]
fn fixed_and_fully_named_loops_still_count_elements() {
    let src = r#"
module tb;
  int m [3][3][$];
  int f [3][4][2];
  initial begin
    int n;
    m[2][0].push_back(1); m[2][0].push_back(2); m[0][1].push_back(3);
    n = 0; foreach (m[i, j, k]) n++;
    $display("full=%0d", n);
    n = 0; foreach (f[i, , k]) n++;
    $display("fixed=%0d", n);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"full=3"), "{msgs:?}");
    assert!(msgs.contains(&"fixed=6"), "{msgs:?}");
}
