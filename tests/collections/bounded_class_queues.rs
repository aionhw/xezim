//! IEEE 1800-2023 §7.10.5 and §8.25. A bounded queue keeps elements at
//! indices 0 through its declared bound in the owning specialization.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .into_iter()
        .map(|entry| entry.message)
        .collect()
}

fn check_capacity(declarations: &str, setup: &str, queue: &str, capacity: u32) {
    let src = format!(
        r#"
module top;
  {declarations}
  initial begin
    {setup}
    for (int i = 10; i < 20; i++) {queue}.push_back(i);
    $display("size=%0d first=%0d last=%0d", {queue}.size(), {queue}[0], {queue}[$]);
  end
endmodule
"#
    );
    assert_eq!(
        run(&src),
        [format!("size={capacity} first=10 last={}", 9 + capacity)]
    );
}

#[test]
fn constant_static_bound() {
    check_capacity("class C; static int q[$:2]; endclass", "", "C::q", 3);
}

#[test]
fn specialized_static_bound() {
    check_capacity(
        "class C #(int N=1); static bit [7:0] q[$:N]; endclass",
        "",
        "C#(3)::q",
        4,
    );
}

#[test]
fn specialized_instance_bound() {
    check_capacity(
        "class C #(int N=1); int q[$:N]; endclass C#(3) c;",
        "c = new;",
        "c.q",
        4,
    );
}

#[test]
fn expression_instance_bound() {
    check_capacity(
        "class C #(int N=1); int q[$:N+1]; endclass C#(3) c;",
        "c = new;",
        "c.q",
        5,
    );
}

#[test]
fn derived_static_bound() {
    check_capacity(
        "class B #(int N=1); static int q[$:N]; endclass
         class D #(int M=1) extends B#(M+1); endclass",
        "",
        "D#(2)::q",
        4,
    );
}

#[test]
fn derived_instance_bound() {
    check_capacity(
        "class B #(int N=1); int q[$:N]; endclass
         class D #(int M=1) extends B#(M+1); endclass D#(2) d;",
        "d = new;",
        "d.q",
        4,
    );
}

#[test]
fn static_bound_in_typedef() {
    check_capacity(
        "class C #(int N=1); typedef int Q[$:N]; static Q q; endclass",
        "",
        "C#(3)::q",
        4,
    );
}

#[test]
fn instance_bound_in_typedef() {
    check_capacity(
        "class C #(int N=1); typedef int Q[$:N]; Q q; endclass C#(3) c;",
        "c = new;",
        "c.q",
        4,
    );
}

#[test]
fn bound_in_typedef_alias() {
    check_capacity(
        "class C #(int N=2); typedef int Q[$:N]; typedef Q R; static R q; endclass",
        "",
        "C#(5)::q",
        6,
    );
}

#[test]
fn class_body_localparam_bound() {
    check_capacity(
        "class C #(int N=2); localparam int B=N+1; static int q[$:B]; endclass",
        "",
        "C#(5)::q",
        7,
    );
}

#[test]
fn like_named_static_queues_keep_independent_bounds() {
    check_capacity(
        "class A; static int q[$:1]; endclass class B; static int q[$:3]; endclass",
        "A::q.push_back(77); A::q.push_back(88); A::q.push_back(99);",
        "B::q",
        4,
    );
}

#[test]
fn local_queue_bound() {
    check_capacity("", "int q[$:2];", "q", 3);
}

#[test]
fn bounded_member_of_heap_owned_struct() {
    check_capacity(
        "typedef struct { int q[$:2]; } S; class C; S s[2]; endclass C c;",
        "c = new;",
        "c.s[1].q",
        3,
    );
}

#[test]
fn full_queue_inserts_trim_the_tail_and_evaluate_the_input_first() {
    let lines = run(r#"
module top;
  int q[$:2];
  int calls;
  function int pick();
    calls++;
    return q[1];
  endfunction
  initial begin
    q = '{10,20,30,40};
    $display("assign %0d %0d", q.size(), q[$]);
    q.push_front(pick());
    $display("front %0d %0d %0d %0d", calls, q[0], q[1], q[2]);
    q.insert(0, pick());
    $display("insert %0d %0d %0d %0d", calls, q[0], q[1], q[2]);
    q.push_back(pick());
    $display("back %0d %0d %0d %0d", calls, q[0], q[1], q[2]);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "assign 3 30",
            "front 1 20 10 20",
            "insert 2 10 20 10",
            "back 3 10 20 10"
        ]
    );
}
