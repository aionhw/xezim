//! §7.10.2.2, §10.10: `push_back`, `push_front` and `insert` on a queue
//! whose elements are queues or dynamic arrays store their argument as the
//! new row. A concatenation `{1, 2}` or a pattern `'{3, 4, 5}` argument is
//! an unpacked-array value of the element type, so the row gets its items,
//! and a queue operand (`{tmp, 3}`) contributes all of its elements.
//! Expected values were checked on the reference simulator.

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
fn push_back_concatenation_and_pattern_into_module_queue() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  q_t qq[$];
  initial begin
    qq.push_back({1, 2});
    qq.push_back('{3, 4, 5});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[0][0], qq[0][1], qq[1][2]);
  end
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=2 s1=3 v=1 2 5"]);
}

#[test]
fn push_front_and_insert_concatenation() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  q_t qq[$];
  initial begin
    qq.push_front({1, 2});
    qq.insert(1, {7, 8, 9});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[0][1], qq[1][2]);
  end
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=2 s1=3 v=2 9"]);
}

#[test]
fn queue_operand_splices_into_the_row() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  q_t qq[$];
  q_t tmp;
  initial begin
    tmp = {1, 2};
    qq.push_back({tmp, 3});
    qq.push_back({4});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[0][0], qq[0][1], qq[0][2]);
  end
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=3 s1=1 v=1 2 3"]);
}

#[test]
fn dynamic_array_rows_and_anonymous_queue_of_queues() {
    let lines = run(r#"
module t;
  typedef int d_t[];
  d_t qd[$];
  int qa[$][$];
  initial begin
    qd.push_back({1, 2});
    qa.push_back({4, 5, 6});
    $display("R s0=%0d s1=%0d v=%0d %0d", qd[0].size(), qa[0].size(), qd[0][1], qa[0][2]);
  end
endmodule
"#);
    assert_eq!(lines, ["R s0=2 s1=3 v=2 6"]);
}

#[test]
fn class_property_queue_of_queues() {
    let lines = run(r#"
class c;
  int qq[$][$];
  function void run();
    qq.push_back({1, 2});
    qq.push_back('{3, 4, 5});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[0][1], qq[1][2]);
  endfunction
endclass
module t;
  initial begin
    automatic c o = new();
    o.run();
  end
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=2 s1=3 v=2 5"]);
}

#[test]
fn task_local_queue_of_queues() {
    let lines = run(r#"
module t;
  task automatic run();
    int l[$][$];
    l.push_back({1, 2});
    $display("R n=%0d s0=%0d v=%0d", l.size(), l[0].size(), l[0][1]);
  endtask
  initial run();
endmodule
"#);
    assert_eq!(lines, ["R n=1 s0=2 v=2"]);
}

/// Guard: pushing a queue variable already copied its elements.
#[test]
fn push_back_queue_variable_guard() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  q_t qq[$];
  q_t tmp;
  initial begin
    tmp = {1, 2};
    qq.push_back(tmp);
    $display("R n=%0d s0=%0d v=%0d %0d", qq.size(), qq[0].size(), qq[0][0], qq[0][1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R n=1 s0=2 v=1 2"]);
}
