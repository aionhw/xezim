//! §10.9.1: each ordered item initializes a row of the destination collection.

#[test]
fn nested_patterns_keep_dynamic_and_queue_rows() {
    let result = xezim::simulate(
        r#"
typedef int queue_t[$];
class C;
  int rows[][];
  function void fill();
    rows = '{'{1, 2}, '{3}};
  endfunction
endclass
module tb;
  int rows[][];
  queue_t queues[$];
  C obj;
  initial begin
    obj = new();
    obj.fill();
    rows = '{'{4, 5}, '{6}};
    queues = '{ {1}, queue_t'{2, 3, 4}, {5, 6} };
    $display("class %0d %0d %0d %0d %0d %0d",
             obj.rows.size(), obj.rows[0].size(), obj.rows[1].size(),
             obj.rows[0][0], obj.rows[0][1], obj.rows[1][0]);
    $display("module %0d %0d %0d %0d %0d %0d",
             rows.size(), rows[0].size(), rows[1].size(),
             rows[0][0], rows[0][1], rows[1][0]);
    $display("queue %0d %0d %0d %0d %0d %0d %0d",
             queues.size(), queues[0].size(), queues[1].size(),
             queues[2].size(), queues[1][0], queues[1][2], queues[2][1]);
  end
endmodule"#,
        10,
    )
    .expect("simulate nested patterns");
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(
        lines,
        [
            "class 2 2 1 1 2 3",
            "module 2 2 1 4 5 6",
            "queue 3 1 3 2 2 4 6"
        ]
    );
}
