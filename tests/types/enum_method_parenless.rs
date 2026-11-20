//! §13.5.5 / §6.19.5.7 — the empty parentheses of a zero-argument call may be
//! omitted, so `e.last` is exactly `e.last()`. The with-parens form always
//! worked; the parenless one reached the enum-method machinery in neither
//! parser shape (issue #233):
//!
//!   * at module/initial/always scope it parses as a flat hierarchical Ident
//!     (`Ident([e, last])`), which read as unknown hierarchical storage -> X;
//!   * in a function/task/class-method/final body it parses as
//!     `MemberAccess{Ident(e), last}`, and `eval_expr_member_access` had no
//!     enum-method arm, so it fell through to the object-property tail -> 0.
//!
//! `.name` additionally had no `BuiltinM::classify` arm at all, so it could
//! never resolve through `eval_builtin_method`, and an enum-typed PROCEDURAL
//! LOCAL was not recognised as an enum receiver because `type_name_of_var`
//! does not consult the typedef maps.
//!
//! The real-world shape is the walk in the last test: `while (1) e = e.next;`
//! exiting on `e == e.last`. With both sides reading X the comparison never
//! held and the simulation hung.

use xezim::simulate;

const E: &str = "typedef enum logic [5:0] { C0 = 6'd0, C1 = 6'd1, CC = 6'd63 } e_t;\n";

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100000).expect("sim");
    sim.output
        .iter()
        .filter(|o| o.message.starts_with("T "))
        .map(|o| o.message.clone())
        .collect()
}

/// Module-scope variable and procedural local, every method, no parentheses.
#[test]
fn parenless_enum_methods_at_statement_scope() {
    let src = format!(
        "module t;\n{E}  e_t m = C1;\n\
  initial begin\n\
    e_t l = C1;\n\
    $display(\"T mod %0d %0d %0d %0d %0d %0s\", m.first, m.last, m.next, m.prev, m.num, m.name);\n\
    $display(\"T loc %0d %0d %0d %0d %0d %0s\", l.first, l.last, l.next, l.prev, l.num, l.name);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec!["T mod 0 63 63 0 3 C1", "T loc 0 63 63 0 3 C1"],
        "a parenless enum method must equal its with-parens form"
    );
}

/// The MemberAccess shape: function, task, class-method and final bodies.
#[test]
fn parenless_enum_methods_in_subroutine_bodies() {
    let src = format!(
        "module t;\n{E}\
  class C; function int fm(); automatic e_t x = C1; return x.last; endfunction endclass\n\
  function int ff(); automatic e_t x = C1; return x.last; endfunction\n\
  task tt(); automatic e_t x = C1; $display(\"T task %0d\", x.last); endtask\n\
  initial begin\n\
    C c = new();\n\
    $display(\"T func %0d\", ff());\n\
    tt();\n\
    $display(\"T meth %0d\", c.fm());\n\
  end\n\
  final begin automatic e_t x = C1; $display(\"T final %0d\", x.last); end\n\
endmodule"
    );
    let out = lines(&src);
    assert!(
        out.contains(&"T func 63".to_string()),
        "function body: {out:?}"
    );
    assert!(out.contains(&"T task 63".to_string()), "task body: {out:?}");
    assert!(
        out.contains(&"T meth 63".to_string()),
        "class method: {out:?}"
    );
    assert!(
        out.contains(&"T final 63".to_string()),
        "final block: {out:?}"
    );
}

/// A formal parameter is a receiver too.
#[test]
fn parenless_enum_method_on_a_formal() {
    let src = format!(
        "module t;\n{E}\
  function int f(e_t p); return p.last; endfunction\n\
  initial $display(\"T formal %0d\", f(C1));\n\
endmodule"
    );
    assert_eq!(lines(&src), vec!["T formal 63"]);
}

/// §5.13, §6.19.5: a class property, a property through a handle or `this`,
/// an array element and a structure member take the parenless methods from
/// their declared type, in a class method and at module scope. A property
/// named `name` stays a property read.
#[test]
fn parenless_enum_methods_on_properties_elements_and_members() {
    let src = format!(
        "module t;\n{E}\
  typedef struct {{ e_t f; int k; }} s_t;\n\
  class N; string name = \"obj\"; endclass\n\
  class C;\n\
    e_t p = C1;\n\
    e_t a[2] = '{{C0, CC}};\n\
    s_t s = '{{f: CC, k: 1}};\n\
    N h = new;\n\
    function void show();\n\
      string n = p.name;\n\
      $display(\"T prop %0s %0d %0d %0d %0d %0d\", n, p.first, p.last, p.next, p.prev, p.num);\n\
      $display(\"T this %0s %0s\", this.p.name, $sformatf(\"%0s\", p.name));\n\
      $display(\"T elem %0s %0s\", a[1].name, s.f.name);\n\
      $display(\"T guard %0s\", h.name);\n\
    endfunction\n\
  endclass\n\
  s_t ms = '{{f: C1, k: 2}};\n\
  initial begin\n\
    C c = new;\n\
    c.show();\n\
    $display(\"T mod %0s %0s %0d\", c.p.name, ms.f.name, ms.f.last);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T prop C1 0 63 63 0 3",
            "T this C1 C1",
            "T elem CC CC",
            "T guard obj",
            "T mod C1 C1 63",
        ],
        "a parenless enum method must equal its with-parens form"
    );
}

/// GUARD: the routing must key on the receiver being enum-typed. A struct
/// member that merely SHARES a method name stays an ordinary data read.
#[test]
fn members_sharing_a_method_name_are_not_hijacked() {
    let src = format!(
        "module t;\n{E}\
  typedef struct packed {{ logic [7:0] first; logic [7:0] name; }} s_t;\n\
  s_t s;\n\
  e_t e = CC;\n\
  initial begin\n\
    s.first = 8'd9; s.name = 8'd7;\n\
    $display(\"T struct %0d %0d\", s.first, s.name);\n\
    $display(\"T enum %0d %0s\", e.first, e.name);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec!["T struct 9 7", "T enum 0 CC"],
        "a struct member named `first`/`name` must not dispatch as an enum method"
    );
}

/// The reported hang: walk with `next` and exit on `== last`. Both sides read
/// X before the fix, so the loop never terminated.
#[test]
fn parenless_next_last_walk_terminates() {
    let src = format!(
        "module t;\n{E}\
  initial begin\n\
    e_t e;\n\
    int steps = 0;\n\
    e = e.first;\n\
    while (steps < 10) begin\n\
      if (e == e.last) break;\n\
      e = e.next;\n\
      steps++;\n\
    end\n\
    $display(\"T walk steps=%0d at=%0s\", steps, e.name);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec!["T walk steps=2 at=CC"],
        "the walk must reach the last member and stop"
    );
}
