//! IEEE 1800-2023 §7.4.5, §10.10, §12.7.3, §13.5.1 (#1646).

#[test]
fn a_fixed_input_of_dynamic_rows_retains_values_across_foreach_waits() {
    let src = r#"
module top;
  typedef struct { logic valid; logic [3:0] data; } item_t;
  logic clock = 0;
  always #5 clock = !clock;
  task wait_cycle;
    @(posedge clock);
  endtask
  task automatic read_rows(input item_t rows[2:1][]);
    foreach (rows[2][i]) begin
      wait_cycle();
      $display("R %0d %0d %h %h", $time, i, rows[2][i].data, rows[1][i].data);
    end
    rows[2][0].data = 0;
  endtask
  initial begin
    item_t a[] = '{'{1, 5}, '{1, 12}, '{0, 10}};
    item_t b[] = '{'{0, 0}, '{1, 3}, '{1, 2}};
    wait_cycle();
    read_rows({a, b});
    $display("R caller %h", a[0].data);
    $finish;
  end
endmodule
"#;
    let sim = xezim::simulate(src, 1000).unwrap();
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("R "))
        .collect();
    assert_eq!(
        lines,
        ["R 15 0 5 0", "R 25 1 c 3", "R 35 2 a 2", "R caller 5"]
    );
}
