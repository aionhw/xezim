//! §21.2.1.7: an unpacked row prints as an assignment pattern in `%p`.

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
fn selected_class_rows_print_like_module_rows() {
    check(
        r#"
module top;
  typedef int pair_t[2];
  class K;
    int a[2][2];
    pair_t q[$];
    int d[][2];
    function void show();
      $display("in method: a[0]=%p q[0]=%p", a[0], q[0]);
    endfunction
  endclass
  K k;
  int m[2][2];
  pair_t mq[$];
  string s;
  initial begin
    k = new;
    k.a[0] = '{1, 2}; k.a[1] = '{3, 4};
    k.q.push_back('{5, 6});
    k.d = new[1]; k.d[0] = '{9, 10};
    m[0] = '{7, 8};
    mq.push_back('{11, 12});
    $display("k.a=%p", k.a);
    $display("k.a[0]=%p k.a[1]=%p", k.a[0], k.a[1]);
    $display("k.q=%p k.q[0]=%p", k.q, k.q[0]);
    $display("k.d[0]=%p", k.d[0]);
    $display("m[0]=%p mq[0]=%p", m[0], mq[0]);
    s = $sformatf("%p", k.a[1]);
    $display("sformatf k.a[1]=%s", s);
    k.show();
  end
endmodule
"#,
        &[
            "k.a='{'{1, 2}, '{3, 4}}",
            "k.a[0]='{1, 2} k.a[1]='{3, 4}",
            "k.q='{'{5, 6}} k.q[0]='{5, 6}",
            "k.d[0]='{9, 10}",
            "m[0]='{7, 8} mq[0]='{11, 12}",
            "sformatf k.a[1]='{3, 4}",
            "in method: a[0]='{1, 2} q[0]='{5, 6}",
        ],
    );
}

#[test]
fn descending_and_last_rows_format_once() {
    check(
        r#"
module class_format_neighbor;
  typedef int pair_t[2];
  class Box;
    int data_rows[2][2:1];
    pair_t q[$];
    function void run();
      int idx = 0;
      data_rows[0] = '{31, 30};
      q.push_back('{41, 40});
      $display("FORMAT row=%p tail=%p", data_rows[idx++], q[$]);
      $display("FORMAT idx=%0d",idx);
    endfunction
  endclass
  initial begin
    Box b = new;
    b.run();
  end
endmodule
"#,
        &["FORMAT row='{31, 30} tail='{41, 40}", "FORMAT idx=1"],
    );
}

#[test]
fn invalid_row_formats_declared_defaults_without_repeating_the_index() {
    check(
        r#"
module top;
  class C;
    int r[2][2];
  endclass
  initial begin
    C c = new;
    int index = 9;
    $display("ROW invalid=%p", c.r[index++]);
    $display("ROW index=%0d", index);
  end
endmodule
"#,
        &["ROW invalid='{0, 0}", "ROW index=10"],
    );
}
