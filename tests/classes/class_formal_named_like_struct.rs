//! §13.3 / §13.5 / §8.4: a class-handle variable named like an unpacked
//! struct elsewhere (`item` here, with a member `item`) is still a class
//! handle in its own scope (#258).
//!
//! * Whether an assignment's left-hand side is an unpacked struct was looked
//!   up in `var_decl_types`, keyed by the bare name and shared by every
//!   process. Another process binding a struct formal `item` while a task
//!   waited made the task's `item = active[idx]` a member-wise struct copy:
//!   the handle was not assigned, and the leaf `item.item` went to the
//!   module-level signal map. A module-scope struct `item` did the same to a
//!   class-handle property `item` assigned in a method.
//! * Reading `item.item` through a class-handle `item` looked the flat name
//!   `item.item` up before following the handle, so a module-scope struct
//!   variable `item` (or such a stray leaf) answered instead of the object.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

const COMMON: &str = r#"
class req;
  int len;
  function new(int l); len = l; endfunction
  function int get_len(); return len; endfunction
endclass

class resp;
  req item;
  function new(req r); item = r; endfunction
endclass

typedef struct {
  req item;
} wrap_t;
"#;

/// The reported shape: a struct formal `item` is bound in another process
/// while the task holding the class-handle formal `item` waits.
#[test]
fn handle_assignment_while_a_struct_formal_is_bound() {
    let src = format!(
        "{COMMON}{}",
        r#"
class consumer;
  task run(req r);
    wrap_t w;
    w.item = r;
    #1;
    body(w);
  endtask
  task body(ref wrap_t item);
    #100;
  endtask
endclass

class driver;
  resp active[$];
  int idx;
  task get_next(ref resp item);
    #2;
    item = active[idx];
    idx++;
  endtask
  function int size_of(resp item);
    return item.item.get_len();
  endfunction
  task run();
    resp item;
    for (int k = 0; k < 3; k++) begin
      get_next(item);
      $display("call %0d: len=%0d size_of=%0d", k, item.item.len, size_of(item));
    end
  endtask
endclass

module top;
  initial begin
    consumer c = new();
    driver d = new();
    req a = new(108), b = new(55), e = new(9), x = new(7);
    resp ra = new(a), rb = new(b), re = new(e);
    d.active.push_back(ra);
    d.active.push_back(rb);
    d.active.push_back(re);
    fork
      c.run(x);
      d.run();
    join
  end
endmodule
"#
    );
    assert_eq!(
        messages(&src),
        [
            "call 0: len=108 size_of=108",
            "call 1: len=55 size_of=55",
            "call 2: len=9 size_of=9",
        ]
    );
}

/// A module-scope struct variable `item` does not answer for a method's
/// class-handle formal or local `item`, nor for a class-handle property
/// `item` assigned in the constructor: the closer declaration wins.
#[test]
fn class_handle_shadows_a_module_struct_of_the_same_name() {
    let src = format!(
        "{COMMON}{}",
        r#"
class user;
  function int via_formal(resp item);
    return item.item.get_len();
  endfunction
  function int via_local(resp r);
    resp item = r;
    return item.item.len;
  endfunction
endclass

// `item = r` in the constructor assigns the class-handle property.
class holder;
  req item;
  function new(req r);
    item = r;
  endfunction
endclass

module top;
  wrap_t item;
  initial begin
    user u = new();
    req m = new(1), a = new(108);
    resp ra = new(a);
    holder h = new(a);
    item.item = m;
    $display("formal=%0d local=%0d property=%0d module=%0d", u.via_formal(ra), u.via_local(ra), h.item == null ? -1 : h.item.len, item.item.len);
  end
endmodule
"#
    );
    assert_eq!(
        messages(&src),
        ["formal=108 local=108 property=108 module=1"]
    );
}
