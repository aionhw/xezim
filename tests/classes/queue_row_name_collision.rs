//! §8.4, §7.4.5: a selected property belongs to its receiver, including when a
//! module declares a queue of the same name and dimensions.

#[test]
fn select_queue_rows_on_their_object() {
    let src = r#"
module tb;
  class K;
    int qi[$][2];
  endclass
  K a, b;
  int qi[$][2];
  initial begin
    a = new; b = new;
    a.qi.push_back('{5, 6});
    a.qi.push_back('{7, 8});
    b.qi.push_back('{9, 10});
    qi.push_back('{1, 2});
    $display("before %0d %0d %0d", a.qi[1][0], a.qi[0][1], qi[0][0]);
    a.qi[0][1] = 42;
    $display("after %0d %0d %0d", a.qi[0][1], b.qi[0][1], qi[0][1]);
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).unwrap();
    let lines: Vec<_> = sim.output.iter().map(|line| line.message.trim()).collect();
    assert_eq!(lines, ["before 7 6 1", "after 42 10 2"]);
}
