//! §7.10.1, §12.7.3: reading an element of a row of a queue whose elements
//! are fixed-size arrays, at module scope and through a class handle. In the
//! row select `q[$][1]`, `$` is the queue's last index; a `foreach` over the
//! queue (or a dynamic array of rows) with one loop variable visits each of
//! its current rows. Expected values were checked on Verilator 5.046.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn issue_repro_dollar_and_foreach_over_class_property_rows() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  class K;
    pair_t q[$];
  endclass
  K k;
  int i, s_fe, s_for;
  initial begin
    k = new;
    k.q.push_back('{1, 2});
    k.q.push_back('{3, 4});
    i = 1;
    $display("R k.q[1][1]=%0d k.q[i][1]=%0d k.q[$][1]=%0d", k.q[1][1], k.q[i][1], k.q[$][1]);
    foreach (k.q[j]) s_fe += k.q[j][1];
    for (int j = 0; j < k.q.size(); j++) s_for += k.q[j][1];
    $display("R foreach sum=%0d for sum=%0d", s_fe, s_for);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R k.q[1][1]=4 k.q[i][1]=4 k.q[$][1]=4",
            "R foreach sum=6 for sum=6",
        ]
    );
}

#[test]
fn dollar_in_the_row_select() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  class K;
    pair_t q[$];
    int qi[$][2];
  endclass
  K k;
  pair_t mq[$];
  int mi[$][2];
  initial begin
    k = new;
    k.q.push_back('{1, 2});
    k.q.push_back('{3, 4});
    k.qi.push_back('{5, 6});
    k.qi.push_back('{7, 8});
    mq.push_back('{1, 2});
    mq.push_back('{3, 4});
    mi.push_back('{5, 6});
    $display("R %0d %0d", k.q[$][1], k.qi[$][0]);
    $display("R %0d %0d", mq[$][1], mq[$ - 1][0]);
    $display("R %0d %0d", k.q[$ - 1][1], k.q[k.q.size() - 1][0]);
    mq[$][1] = 9;
    k.q[$][0] = 7;
    mi[$][1] = 10;
    $display("R %0d %0d %0d", mq[$][1], k.q[$][0], mi[$][1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R 4 7", "R 4 1", "R 2 3", "R 9 7 10"]);
}

#[test]
fn foreach_visits_each_current_row() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  class K;
    pair_t q[$];
    int qi[$][2];
    int d[][2];
  endclass
  K k;
  pair_t mq[$];
  int s, n;
  initial begin
    k = new;
    k.q.push_back('{1, 2});
    k.q.push_back('{3, 4});
    k.qi.push_back('{5, 6});
    k.qi.push_back('{7, 8});
    k.d = new[2];
    k.d[1][0] = 9;
    mq.push_back('{1, 2});
    mq.push_back('{3, 4});
    n = 0;
    foreach (k.q[j]) begin
      $display("R j=%0d %0d %0d", j, k.q[j][0], k.q[j][1]);
      n++;
    end
    s = 0;
    foreach (k.qi[j]) s += k.qi[j][1];
    $display("R n=%0d s=%0d", n, s);
    s = 0;
    foreach (k.q[j, m]) s += k.q[j][m];
    $display("R s=%0d", s);
    s = 0;
    foreach (mq[j]) s += mq[j][1];
    $display("R s=%0d", s);
    s = 0;
    foreach (k.d[j]) s += k.d[j][0];
    $display("R s=%0d", s);
    k.q.delete();
    n = 0;
    foreach (k.q[j]) n++;
    $display("R empty n=%0d", n);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R j=0 1 2",
            "R j=1 3 4",
            "R n=2 s=14",
            "R s=10",
            "R s=6",
            "R s=9",
            "R empty n=0",
        ]
    );
}
