//! §13.3 / §13.4: a formal is a variable of its subroutine's own scope with
//! its declared type. A formal's class/typedef type belongs in the callee
//! frame's type overlay (`local_type_stack`), pushed with the frame:
//!   * the task path recorded it only in the simulator-global map keyed by
//!     the bare name, so while another process sat parked in a task
//!     declaring a same-named local, that local's class won — after the
//!     task resumed, `$cast(item, src)` checked `src` against the other
//!     local's class and failed (#239);
//!   * the function and method paths recorded it before pushing the callee
//!     frame, i.e. into the CALLER's overlay, so a caller's same-named local
//!     took the formal's type after the call returned.
//!
//! The task path also bound its formals before the callee's class context
//! was in force, so a formal typed by a class type parameter bound to an
//! unpacked struct (§8.25) was not bound member-wise and read x.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

/// Each call runs while a freshly forked `other_user::run` (local
/// `other item;`) is parked.
const PARKED: &str = r#"
class base; endclass
class pkt extends base; endclass
class other; endclass

class consumer;
  task get_out(base src, output pkt item);
    #1;
    if (!$cast(item, src)) $display("get_out: cast failed");
  endtask
  task get_ref(base src, ref pkt item);
    #1;
    if (!$cast(item, src)) $display("get_ref: cast failed");
  endtask
  static task get_static(base src, output pkt item);
    #1;
    if (!$cast(item, src)) $display("get_static: cast failed");
  endtask
endclass

virtual class waiter_base #(type ITEM = base);
  virtual task get(base src, ref ITEM item);
    #1;
    if (!$cast(item, src)) $display("waiter: cast failed");
  endtask
endclass
class waiter extends waiter_base #(.ITEM(pkt)); endclass

class other_user;
  task run();
    other item;
    #100;
  endtask
endclass

module top;
  task automatic get_mod(base src, output pkt item);
    #1;
    if (!$cast(item, src)) $display("get_mod: cast failed");
  endtask

  initial begin
    consumer c = new();
    waiter w = new();
    other_user u = new();
    pkt p = new(), r1, r2, r3, r4, r5;
    fork u.run(); join_none
    c.get_out(p, r1);
    fork u.run(); join_none
    c.get_ref(p, r2);
    fork u.run(); join_none
    consumer::get_static(p, r3);
    fork u.run(); join_none
    w.get(p, r4);
    fork u.run(); join_none
    get_mod(p, r5);
    $display("out=%0d ref=%0d static=%0d type_param=%0d module=%0d",
             r1 == p, r2 == p, r3 == p, r4 == p, r5 == p);
    $finish;
  end
endmodule
"#;

#[test]
fn parked_task_formal_keeps_its_declared_class() {
    let out = messages(PARKED);
    assert_eq!(
        out,
        ["out=1 ref=1 static=1 type_param=1 module=1"],
        "{out:?}"
    );
}

/// The caller's local `x` keeps its class `other` after calling a
/// subroutine whose formal `x` is a `pkt`.
const CALLER: &str = r#"
class pkt; endclass
class other; endclass

class helper;
  function void f(pkt x); endfunction
  task t(pkt x); endtask
  static function void sf(pkt x); endfunction
endclass

function automatic void pf(pkt x); endfunction

class user;
  task run();
    other x;
    other o = new();
    helper h = new();
    int ok_f, ok_t, ok_sf, ok_pf;
    h.f(null);
    ok_f = $cast(x, o);
    h.t(null);
    ok_t = $cast(x, o);
    helper::sf(null);
    ok_sf = $cast(x, o);
    pf(null);
    ok_pf = $cast(x, o);
    $display("function=%0d task=%0d static=%0d plain=%0d", ok_f, ok_t, ok_sf, ok_pf);
  endtask
endclass

module top;
  initial begin
    user u = new();
    u.run();
  end
endmodule
"#;

#[test]
fn callee_formal_type_does_not_leak_into_caller_frame() {
    let out = messages(CALLER);
    assert_eq!(out, ["function=1 task=1 static=1 plain=1"], "{out:?}");
}

/// A blocking class task's formal typed by a type parameter bound to an
/// unpacked struct, called from a class whose own `T` is another type.
const TYPE_PARAM_STRUCT: &str = r#"
typedef struct { int a; int b; } pair_t;

class holder #(type T = int);
  task show(T t);
    #1;
    $display("in=%0d,%0d", t.a, t.b);
  endtask
  task make(output T t);
    #1;
    t.a = 5;
    t.b = 6;
  endtask
  static task show_static(T t);
    #1;
    $display("static=%0d,%0d", t.a, t.b);
  endtask
endclass

// The caller's own `T` is a different type.
class caller #(type T = int);
  task run(holder #(pair_t) h, pair_t p);
    pair_t r;
    h.show(p);
    h.make(r);
    $display("out=%0d,%0d", r.a, r.b);
    holder #(pair_t)::show_static(p);
  endtask
endclass

module top;
  initial begin
    holder #(pair_t) h = new();
    caller #(int) c = new();
    pair_t p;
    p = '{1, 2};
    c.run(h, p);
  end
endmodule
"#;

#[test]
fn blocking_task_type_param_struct_formal_binds_member_wise() {
    let out = messages(TYPE_PARAM_STRUCT);
    assert_eq!(out, ["in=1,2", "out=5,6", "static=1,2"], "{out:?}");
}
