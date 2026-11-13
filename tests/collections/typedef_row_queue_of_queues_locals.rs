//! §7.4.5, §6.18: a local declared with an unpacked-array typedef as its
//! element type and dimensions of its own (`typedef int q_t[$]; q_t l[$];`)
//! is a collection of collections: the declarator's dimensions come first,
//! then the typedef's. The typedef's dimensions were dropped whenever the
//! declarator had any, so each row held one scalar and a pushed queue lost
//! its items. Expected values were checked on the reference simulator.

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
fn task_local_rows_of_a_module_typedef() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  task automatic run();
    q_t tmp;
    q_t l[$];
    tmp = {1, 2};
    l.push_back(tmp);
    l.push_back({3, 4, 5});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d", l.size(), l[0].size(), l[1].size(), l[0][1], l[1][2]);
  endtask
  initial run();
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=2 s1=3 v=2 5"]);
}

#[test]
fn unit_typedef_and_dynamic_array_rows() {
    let lines = run(r#"
typedef int uq_t[$];
module t;
  typedef int d_t[];
  task automatic run();
    uq_t l[$];
    d_t ld[$];
    l.push_back({7, 8});
    ld.push_back({4, 5, 6});
    $display("R s0=%0d v=%0d d0=%0d dv=%0d", l[0].size(), l[0][1], ld[0].size(), ld[0][2]);
  endtask
  initial run();
endmodule
"#);
    assert_eq!(lines, ["R s0=2 v=8 d0=3 dv=6"]);
}

#[test]
fn class_method_local_rows_of_a_class_typedef() {
    let lines = run(r#"
class c;
  typedef int q_t[$];
  function void run();
    q_t l[$];
    l.push_back({1, 2});
    l[0].push_back(9);
    $display("R n=%0d s0=%0d v=%0d %0d", l.size(), l[0].size(), l[0][1], l[0][2]);
  endfunction
endclass
module t;
  initial begin
    automatic c o = new();
    o.run();
  end
endmodule
"#);
    assert_eq!(lines, ["R n=1 s0=3 v=2 9"]);
}

/// Guards: the anonymous form and a typedef'd local without dimensions of
/// its own (one level) were already right.
#[test]
fn anonymous_and_single_level_locals_guard() {
    let lines = run(r#"
module t;
  typedef int q_t[$];
  task automatic run();
    int la[$][$];
    q_t one;
    la.push_back({5, 6});
    one = {1, 2, 3};
    $display("R s0=%0d v=%0d one=%0d", la[0].size(), la[0][1], one.size());
  endtask
  initial run();
endmodule
"#);
    assert_eq!(lines, ["R s0=2 v=6 one=3"]);
}
