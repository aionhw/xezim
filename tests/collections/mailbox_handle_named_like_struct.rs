//! §8.25 / §13.3 / §15.4: a class handle sent through a mailbox from a
//! method of a parameterized class reaches the receiver even while another
//! process is blocked in a different specialization whose same-named formal
//! is an unpacked struct (#260).
//!
//! Whether a message is an unpacked struct was decided from `var_decl_types`,
//! keyed by the bare name and shared by every process. A process blocked in
//! `fifo #(msg_t)::get(output T t)` left `t` registered as `msg_t`, and the
//! formal `t` of `fifo #(item)::try_put` (typed by the type parameter) was not
//! recognised as a class handle, so the handle was stashed as a struct message
//! and the receiver got the stash id instead of the object.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("sim");
    sim.output
        .iter()
        .filter(|o| o.message.starts_with("T "))
        .map(|o| o.message.clone())
        .collect()
}

const COMMON: &str = r#"
class item;
  int n;
  function new(int n); this.n = n; endfunction
endclass

typedef struct {
  int k;
} msg_t;

class fifo #(type T = int);
  mailbox #(T) m = new();
  task get(output T t);
    m.get(t);
  endtask
  task put(T t);
    m.put(t);
  endtask
  function bit try_put(T t);
    return m.try_put(t);
  endfunction
endclass
"#;

/// The reported shape: `try_put` from `fifo #(item)` while a process waits
/// in `fifo #(msg_t)::get`. The struct receiver still gets its member.
#[test]
fn try_put_handle_while_struct_specialization_is_blocked() {
    let src = format!(
        "{COMMON}{}",
        r#"
module top;
  fifo #(item)  fc = new();
  fifo #(msg_t) fs = new();
  initial begin
    item it;
    fc.get(it);
    $display("T handle n=%0d", it == null ? -1 : it.n);
  end
  initial begin
    msg_t m;
    fs.get(m);
    $display("T struct k=%0d", m.k);
  end
  initial begin
    item x;
    msg_t s;
    #1;
    x = new(1);
    void'(fc.try_put(x));
    #1;
    s.k = 7;
    void'(fs.try_put(s));
  end
endmodule
"#
    );
    assert_eq!(lines(&src), vec!["T handle n=1", "T struct k=7"]);
}

/// The blocking `put` task takes the same message path.
#[test]
fn put_handle_while_struct_specialization_is_blocked() {
    let src = format!(
        "{COMMON}{}",
        r#"
module top;
  fifo #(item)  fc = new();
  fifo #(msg_t) fs = new();
  initial begin
    item it;
    fc.get(it);
    $display("T handle n=%0d", it == null ? -1 : it.n);
  end
  initial begin
    msg_t m;
    fs.get(m);
    $display("T struct k=%0d", m.k);
  end
  initial begin
    item x;
    msg_t s;
    #1;
    x = new(2);
    fc.put(x);
    #1;
    s.k = 8;
    fs.put(s);
  end
endmodule
"#
    );
    assert_eq!(lines(&src), vec!["T handle n=2", "T struct k=8"]);
}

/// The fifo lives in a base class handed over as a type parameter, the way
/// `uvm_sequencer #(REQ, RSP)` is passed as `BASE`: the fifo's `T` is bound
/// to the enclosing class's `REQ`, which must still resolve to `item`.
#[test]
fn try_put_handle_through_a_base_class_type_parameter() {
    let src = format!(
        "{COMMON}{}",
        r#"
class base #(type REQ = int);
  fifo #(REQ) f;
  function new();
    f = new();
  endfunction
endclass

class wrap #(type BASE = int) extends BASE;
endclass

class sqr #(type REQ = int) extends wrap #(.BASE(base #(REQ)));
endclass

typedef sqr #(item) item_sqr;

class my_sqr extends wrap #(.BASE(item_sqr));
endclass

module top;
  my_sqr        s = new();
  fifo #(msg_t) fs = new();
  initial begin
    item it;
    s.f.get(it);
    $display("T handle n=%0d", it == null ? -1 : it.n);
  end
  initial begin
    msg_t m;
    fs.get(m);
    $display("T struct k=%0d", m.k);
  end
  initial begin
    item x;
    msg_t s2;
    #1;
    x = new(3);
    void'(s.f.try_put(x));
    #1;
    s2.k = 9;
    void'(fs.try_put(s2));
  end
endmodule
"#
    );
    assert_eq!(lines(&src), vec!["T handle n=3", "T struct k=9"]);
}
