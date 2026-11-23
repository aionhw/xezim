//! §7.6, §11.3.2, §11.4.2: selected rows preserve shape and evaluate their index once.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn parenthesized_row_retains_its_shape() {
    check(
        r#"
module tb;
  int m[2][2];
  int r[2];
  class C;
    int m[2][2];
    int r[2];
    function void run();
      m = '{'{1, 2}, '{3, 4}};
      r = '{9, 9};
      r = (m[1]);
      $display("class  r = (m[1]): %0d %0d", r[0], r[1]);
    endfunction
  endclass
  C c;
  initial begin
    m = '{'{1, 2}, '{3, 4}};
    r = '{9, 9};
    r = (m[1]);
    $display("module r = (m[1]): %0d %0d", r[0], r[1]);
    r = '{9, 9};
    r = m[1];
    $display("module r = m[1]:   %0d %0d", r[0], r[1]);
    c = new;
    c.run();
  end
endmodule
"#,
        &[
            "module r = (m[1]): 3 4",
            "module r = m[1]:   3 4",
            "class  r = (m[1]): 3 4",
        ],
    );
}

#[test]
fn effectful_row_index_runs_once() {
    check(
        r#"
module tb;
  int m[3][2];
  int r[2];
  int i;
  class C;
    int m[3][2];
    int r[2];
    int i;
    function void run();
      m = '{'{1, 2}, '{3, 4}, '{5, 6}};
      r = '{9, 9};
      i = 0;
      r = m[i++];
      $display("class  r = m[i++]: %0d %0d i=%0d", r[0], r[1], i);
    endfunction
  endclass
  C c;
  initial begin
    m = '{'{1, 2}, '{3, 4}, '{5, 6}};
    r = '{9, 9};
    i = 0;
    r = m[i++];
    $display("module r = m[i++]: %0d %0d i=%0d", r[0], r[1], i);
    c = new;
    c.run();
  end
endmodule
"#,
        &["module r = m[i++]: 1 2 i=1", "class  r = m[i++]: 1 2 i=1"],
    );
}

#[test]
fn parenthesized_class_row_calls_selector_once() {
    check(
        r#"
module row_operand_neighbor;
  class Box;
    int cnt;
    int source_rows[2][2];
    int dest_row[2];
    function int pick();
      cnt++;
      return 1;
    endfunction
    function void run();
      source_rows = '{'{10, 20}, '{30, 40}};
      dest_row = ((source_rows[pick()]));
      $display("ROW %p calls=%0d", dest_row, cnt);
    endfunction
  endclass
  initial begin
    Box b = new;
    b.run();
  end
endmodule
"#,
        &["ROW '{30, 40} calls=1"],
    );
}
