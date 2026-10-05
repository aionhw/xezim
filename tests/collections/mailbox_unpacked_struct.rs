//! §15.4 — a `mailbox #(T)` whose message type is an UNPACKED struct.
//!
//! A queue slot holds one `Value`, and an unpacked struct has no single value:
//! its members are separate leaf signals. `put` evaluated the argument to a
//! scalar and `get`/`peek`/`try_peek` assigned that scalar back, so the
//! receiving variable kept whatever members it already had while the box's own
//! bookkeeping (`num()`, removal) looked correct (issue #241). A PACKED struct
//! round-trips through the single value and always worked.
//!
//! Such a message is now copied member-wise into a shadow variable, and the
//! slot carries the shadow's id. Ids are recycled when a message is consumed,
//! so the shadows are bounded by the deepest the box ever got rather than
//! growing with every message.

use xezim::simulate;

const S: &str = "typedef struct { int a; int b; } pair_t;\n";

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("sim");
    sim.output
        .iter()
        .filter(|o| o.message.starts_with("T "))
        .map(|o| o.message.clone())
        .collect()
}

/// The reported case: peek, try_peek and get all deliver the members.
#[test]
fn get_peek_try_peek_deliver_an_unpacked_struct() {
    let src = format!(
        "{S}module t;\n\
  initial begin\n\
    mailbox #(pair_t) mb = new();\n\
    pair_t p, q1, q2, q3;\n\
    p = '{{1, 2}}; q1 = '{{99, 99}}; q2 = '{{99, 99}}; q3 = '{{99, 99}};\n\
    mb.put(p);\n\
    mb.peek(q1);\n\
    void'(mb.try_peek(q2));\n\
    mb.get(q3);\n\
    $display(\"T peek %0d %0d\", q1.a, q1.b);\n\
    $display(\"T tryp %0d %0d\", q2.a, q2.b);\n\
    $display(\"T get %0d %0d\", q3.a, q3.b);\n\
    $display(\"T num %0d\", mb.num());\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec!["T peek 1 2", "T tryp 1 2", "T get 1 2", "T num 0"],
        "a peek must not consume, and all three must deliver the members"
    );
}

/// Several messages in flight at once: the shadow ids must not alias, so FIFO
/// order and per-message contents both have to hold.
#[test]
fn several_queued_messages_keep_their_own_members() {
    let src = format!(
        "{S}module t;\n\
  initial begin\n\
    mailbox #(pair_t) mb = new();\n\
    pair_t p, q;\n\
    for (int i = 0; i < 4; i++) begin p = '{{i, i * 10}}; mb.put(p); end\n\
    $display(\"T num %0d\", mb.num());\n\
    for (int i = 0; i < 4; i++) begin\n\
      q = '{{99, 99}};\n\
      mb.get(q);\n\
      $display(\"T msg %0d %0d\", q.a, q.b);\n\
    end\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T num 4",
            "T msg 0 0",
            "T msg 1 10",
            "T msg 2 20",
            "T msg 3 30"
        ]
    );
}

/// Ids are recycled, so a long run reuses shadows rather than accumulating
/// them. Correctness after many put/get cycles is the observable part.
#[test]
fn ids_recycle_across_many_put_get_cycles() {
    let src = format!(
        "{S}module t;\n\
  initial begin\n\
    mailbox #(pair_t) mb = new();\n\
    pair_t p, q;\n\
    int bad = 0;\n\
    for (int i = 0; i < 50; i++) begin\n\
      p = '{{i, i + 1}};\n\
      mb.put(p);\n\
      q = '{{99, 99}};\n\
      mb.get(q);\n\
      if (q.a != i || q.b != i + 1) bad++;\n\
    end\n\
    $display(\"T cycles bad=%0d num=%0d\", bad, mb.num());\n\
  end\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T cycles bad=0 num=0"]);
}

/// A struct member that is a CLASS HANDLE travels too.
#[test]
fn a_class_handle_member_is_delivered() {
    let src = "\
class obj; int v; endclass\n\
typedef struct { int a; obj h; } withh_t;\n\
module t;\n\
  initial begin\n\
    mailbox #(withh_t) mb = new();\n\
    withh_t w, r;\n\
    w.a = 5; w.h = new(); w.h.v = 77;\n\
    mb.put(w);\n\
    r.a = 0; r.h = null;\n\
    mb.get(r);\n\
    $display(\"T handle %0d %0s %0d\", r.a, r.h == null ? \"null\" : \"set\",\n\
             r.h == null ? -1 : r.h.v);\n\
  end\n\
endmodule";
    assert_eq!(lines(src), vec!["T handle 5 set 77"]);
}

/// The BLOCKING path: a consumer parks on the empty box and the message is
/// handed to it by a later `put`, through `deliver_to_mailbox_waiter`.
#[test]
fn a_parked_consumer_receives_the_members() {
    let src = format!(
        "{S}module t;\n\
  mailbox #(pair_t) mb = new();\n\
  initial begin\n\
    pair_t c;\n\
    c = '{{99, 99}};\n\
    mb.get(c);\n\
    $display(\"T blocked %0d %0d at %0t\", c.a, c.b, $time);\n\
  end\n\
  initial begin\n\
    pair_t d;\n\
    #5;\n\
    d = '{{11, 22}};\n\
    mb.put(d);\n\
  end\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T blocked 11 22 at 5"]);
}

/// A mailbox held as a property, including of a PARAMETERIZED class, with the
/// message type coming from the type parameter.
#[test]
fn a_mailbox_property_delivers_an_unpacked_struct() {
    let src = format!(
        "{S}\
class plain; mailbox #(pair_t) mb = new(); endclass\n\
class gen #(type T = int); mailbox #(T) mb = new(); endclass\n\
module t;\n\
  initial begin\n\
    plain a = new();\n\
    gen #(pair_t) g = new();\n\
    pair_t p, q;\n\
    p = '{{3, 4}}; q = '{{99, 99}};\n\
    a.mb.put(p); a.mb.get(q);\n\
    $display(\"T plain %0d %0d\", q.a, q.b);\n\
    p = '{{5, 6}}; q = '{{99, 99}};\n\
    g.mb.put(p); g.mb.get(q);\n\
    $display(\"T param %0d %0d\", q.a, q.b);\n\
  end\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T plain 3 4", "T param 5 6"]);
}

/// CONTROL: a PACKED struct rides in the queued value and must be unaffected.
#[test]
fn a_packed_struct_still_round_trips() {
    let src = "\
typedef struct packed { logic [31:0] a; logic [31:0] b; } ppair_t;\n\
module t;\n\
  initial begin\n\
    mailbox #(ppair_t) mb = new();\n\
    ppair_t p, q;\n\
    p = '{1, 2}; q = '{99, 99};\n\
    mb.put(p);\n\
    mb.get(q);\n\
    $display(\"T packed %0d %0d\", q.a, q.b);\n\
  end\n\
endmodule";
    assert_eq!(lines(src), vec!["T packed 1 2"]);
}

/// The message passes through a FORMAL of the task that calls `put` / `get`
/// — a FIFO class wrapping its mailbox, plain or with the message type from
/// a type parameter (issue #243). Inside a task body the calls reach the
/// expression-statement mailbox path, which queued and assigned the formal
/// as one scalar.
#[test]
fn task_formals_carry_an_unpacked_struct() {
    let src = format!(
        "{S}\
class fifo;\n\
  mailbox #(pair_t) mb = new();\n\
  task put(pair_t t); mb.put(t); endtask\n\
  task get(output pair_t t); mb.get(t); endtask\n\
  task peek(output pair_t t); mb.peek(t); endtask\n\
endclass\n\
class gfifo #(type T = int);\n\
  local mailbox #(T) mb = new();\n\
  task put(T t); mb.put(t); endtask\n\
  task get(output T t); mb.get(t); endtask\n\
endclass\n\
module t;\n\
  initial begin\n\
    fifo f = new();\n\
    gfifo #(pair_t) g = new();\n\
    pair_t p, q;\n\
    p = '{{1, 2}}; q = '{{99, 99}};\n\
    f.put(p); f.mb.get(q);\n\
    $display(\"T put %0d %0d\", q.a, q.b);\n\
    p = '{{3, 4}}; q = '{{99, 99}};\n\
    f.mb.put(p); f.peek(q);\n\
    $display(\"T peek %0d %0d\", q.a, q.b);\n\
    q = '{{99, 99}};\n\
    f.get(q);\n\
    $display(\"T get %0d %0d num %0d\", q.a, q.b, f.mb.num());\n\
    p = '{{5, 6}}; q = '{{99, 99}};\n\
    g.put(p); g.get(q);\n\
    $display(\"T param %0d %0d\", q.a, q.b);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec!["T put 1 2", "T peek 3 4", "T get 3 4 num 0", "T param 5 6"]
    );
}

/// Through task formals on the blocking paths: a `get` parked on the empty
/// box takes the message handed over by a later `put`, and a `put` parked on
/// a full bounded box queues its message once a slot frees.
#[test]
fn task_formals_on_the_blocking_paths() {
    let src = format!(
        "{S}\
class fifo;\n\
  mailbox #(pair_t) mb;\n\
  function new(int bound); mb = new(bound); endfunction\n\
  task put(pair_t t); mb.put(t); endtask\n\
  task get(output pair_t t); mb.get(t); endtask\n\
endclass\n\
module t;\n\
  fifo e = new(0);\n\
  fifo b = new(1);\n\
  initial begin\n\
    pair_t c;\n\
    c = '{{99, 99}};\n\
    e.get(c);\n\
    $display(\"T parked get %0d %0d at %0t\", c.a, c.b, $time);\n\
  end\n\
  initial begin\n\
    pair_t d;\n\
    #5;\n\
    d = '{{11, 22}};\n\
    e.put(d);\n\
  end\n\
  initial begin\n\
    pair_t p;\n\
    #10;\n\
    p = '{{1, 2}}; b.put(p);\n\
    p = '{{3, 4}}; b.put(p);\n\
    $display(\"T parked put done at %0t\", $time);\n\
  end\n\
  initial begin\n\
    pair_t q1, q2;\n\
    #20;\n\
    q1 = '{{99, 99}}; b.get(q1);\n\
    q2 = '{{99, 99}}; b.get(q2);\n\
    #1;\n\
    $display(\"T bounded %0d %0d, %0d %0d\", q1.a, q1.b, q2.a, q2.b);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T parked get 11 22 at 5",
            "T parked put done at 20",
            "T bounded 1 2, 3 4"
        ]
    );
}

/// Function formals with the non-blocking calls.
#[test]
fn function_formals_with_try_put_try_get() {
    let src = format!(
        "{S}\
class fifo;\n\
  mailbox #(pair_t) mb = new();\n\
  function bit put(pair_t t); return mb.try_put(t); endfunction\n\
  function bit get(output pair_t t); return mb.try_get(t); endfunction\n\
endclass\n\
module t;\n\
  initial begin\n\
    fifo f = new();\n\
    pair_t p, q;\n\
    p = '{{7, 8}}; q = '{{99, 99}};\n\
    void'(f.put(p)); void'(f.get(q));\n\
    $display(\"T try %0d %0d num %0d\", q.a, q.b, f.mb.num());\n\
  end\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T try 7 8 num 0"]);
}

/// Consuming a message frees a stash id only when that message IS a stashed
/// struct. A scalar message whose value happened to equal a queued struct's
/// id freed that id, so the next struct `put` reused the slot and overwrote
/// the message still waiting in the box.
#[test]
fn a_scalar_message_does_not_free_a_queued_struct_slot() {
    let src = format!(
        "{S}\
class ififo;\n\
  mailbox #(int) mb = new();\n\
  task put(int v); mb.put(v); endtask\n\
  task get(output int v); mb.get(v); endtask\n\
endclass\n\
module t;\n\
  initial begin\n\
    mailbox #(pair_t) ms = new();\n\
    mailbox #(int) mi = new();\n\
    ififo f = new();\n\
    pair_t p, q;\n\
    int n;\n\
    p = '{{1, 2}}; ms.put(p);\n\
    mi.put(1); mi.get(n);\n\
    f.put(1); f.get(n);\n\
    p = '{{3, 4}}; ms.put(p);\n\
    p = '{{5, 6}}; ms.put(p);\n\
    for (int i = 0; i < 3; i++) begin\n\
      q = '{{99, 99}}; ms.get(q);\n\
      $display(\"T msg %0d %0d\", q.a, q.b);\n\
    end\n\
  end\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T msg 1 2", "T msg 3 4", "T msg 5 6"]);
}
