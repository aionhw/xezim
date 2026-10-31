//! §7.10.4: a concatenation assigned to a string queue is a queue
//! assignment, also when the queue is a class property written inside a
//! method. `q = {}` empties it and `q = {q, "z"}` appends. A string queue
//! is also marked as a string, so the method form was a byte concatenation
//! and kept the old elements.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn class_string_queue_assigned_a_concatenation_in_a_method() {
    let out = lines(
        r#"
class b;
  string q[$] = {"base"};
endclass
class d extends b;
  string own[$] = {"own"};
  function void clear(); q = {}; own = {}; endfunction
  function void set2(); q = {"x", "y"}; own = {"p", "q", "r"}; endfunction
  function void app(); q = {q, "z"}; own = {own, "s"}; endfunction
  function void loc(); string q; q = {"a", "b"}; $display("loc %s", q); endfunction
endclass
module t;
  string mq[$] = {"m"};
  initial begin
    d o = new();
    o.clear();
    mq = {};
    $display("clear q=%0d own=%0d mq=%0d", o.q.size(), o.own.size(), mq.size());
    o.set2();
    $display("set2 q=%0d own=%0d %s %s", o.q.size(), o.own.size(), o.q[0], o.own[2]);
    o.loc();
    o.app();
    $display("app q=%0d own=%0d %s %s", o.q.size(), o.own.size(), o.q[2], o.own[3]);
  end
endmodule
"#,
    );
    for want in [
        "clear q=0 own=0 mq=0",
        "set2 q=2 own=3 x r",
        "app q=3 own=4 z s",
        "loc ab",
    ] {
        assert!(out.iter().any(|m| m == want), "missing {want:?}: {out:?}");
    }
}
