//! Issues #877, #878, #883, #886 and #887: lexical type and handle lookup
//! (§6.20.2, §6.21, §8.25, §15.4, §18.5, §23.9) and built-in legality.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn run(source: &str, extra: &[&str]) -> (bool, String) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim-scope-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "--no-cache",
            "-s",
            "top",
            "--max-time",
            "100ns",
        ])
        .args(extra)
        .arg(&file)
        .current_dir(&dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).unwrap();
    (out.status.success(), text)
}

#[test]
fn automatic_mailbox_queries_and_aliases_use_the_frame() {
    let (ok, out) = run(
        r#"
module top;
  mailbox #(int) box = new();
  task automatic check();
    mailbox #(int) box = new(2);
    mailbox #(int) alias_box;
    int v;
    box.put(10); box.put(20); alias_box = box;
    $display("A|%0d|%0d|%0d", box.num(), alias_box.num(), box.try_put(30));
    begin
      mailbox #(int) inner = new();
      inner.put(7);
      $display("B|%0d|%0d", inner.num(), box.num());
    end
    alias_box.get(v);
    $display("C|%0d|%0d|%0d", v, box.num(), alias_box.num());
    while (box.num() > 0) box.get(v);
    $display("D|%0d|%0d", box.num(), v);
  endtask
  initial begin box.put(99); check(); $display("E|%0d", box.num()); $finish; end
endmodule
"#,
        &[],
    );
    assert!(ok, "{out}");
    for expected in ["A|2|2|0", "B|1|2", "C|10|1|1", "D|0|20", "E|1"] {
        assert!(out.lines().any(|s| s == expected), "{expected}: {out}");
    }
}

#[test]
fn automatic_object_mode_queries_do_not_change_a_module_twin() {
    let (ok, out) = run(
        r#"
class C;
  rand int x;
  constraint positive { x > 0; }
endclass
module top;
  C obj = new();
  task automatic check();
    C obj = new();
    $display("A|%0d|%0d", obj.x.rand_mode(), obj.positive.constraint_mode());
    obj.x.rand_mode(0); obj.positive.constraint_mode(0);
    $display("B|%0d|%0d", obj.x.rand_mode(), obj.positive.constraint_mode());
  endtask
  initial begin check(); $display("C|%0d|%0d", obj.x.rand_mode(), obj.positive.constraint_mode()); $finish; end
endmodule
"#,
        &[],
    );
    assert!(
        ok && out.contains("A|1|1") && out.contains("B|0|0") && out.contains("C|1|1"),
        "{out}"
    );
}

#[test]
fn inherited_constraints_in_instances_and_enclosing_generate_scopes() {
    let (ok, out) = run(
        r#"
module worker(output bit ok);
  class base; rand int lo; rand int hi; endclass
  class derived extends base; constraint order { lo < hi; hi - lo < 100; } endclass
  derived d = new();
  initial ok = d.randomize() && d.lo < d.hi && d.hi - d.lo < 100;
endmodule
module top;
  class base; rand int n; endclass
  if (1) begin : g
    class derived extends base; constraint bound { n inside {[10:20]}; } endclass
    derived d = new();
    bit ok;
    initial ok = d.randomize() && d.n >= 10 && d.n <= 20;
  end
  bit a, b;
  worker u(.ok(a)), v(.ok(b));
  initial begin #1; $display("C|%0d|%0d|%0d", a, b, g.ok); $finish; end
endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("C|1|1|1"), "{out}");
}

#[test]
fn inherited_constraint_does_not_admit_an_undeclared_member() {
    let (ok, out) = run(
        r#"
module worker;
  class base; rand int lo; endclass
  class derived extends base; constraint bad { lo < no_such_name; } endclass
endmodule
module top; worker u(); endmodule
"#,
        &[],
    );
    assert!(
        !ok && out.contains("Undeclared identifier 'no_such_name'"),
        "{out}"
    );
}

#[test]
fn module_class_hides_unit_class_without_capturing_unit_code() {
    for src in [
        include_str!("../issue_cases/nonvhdl_887/basic.sv"),
        include_str!("../issue_cases/nonvhdl_887/constraint.sv"),
        include_str!("../issue_cases/nonvhdl_887/generate.sv"),
        include_str!("../issue_cases/nonvhdl_887/plain_hides_parameterized.sv"),
        include_str!("../issue_cases/nonvhdl_887/explicit_unit.sv"),
    ] {
        let (ok, out) = run(src, &[]);
        assert!(ok && out.contains("PASS") && !out.contains("FAIL"), "{out}");
    }
}

#[test]
fn class_hiding_reaches_headers_out_of_block_methods_and_specialization_text() {
    let (ok, out) = run(
        r#"
class base; int x=5; endclass
class Wrap #(type T=base);
  static function int get(); T t=new(); return t.x; endfunction
endclass
function automatic base make_unit(); base b=new(); return b; endfunction
module worker #(parameter type Header=base);
  typedef class base;
  class base; int x=7; endclass : base
  class derived extends base; extern function int get(); endclass
  function int derived::get(); return x; endfunction
  class Box #(type T=base);
    T value;
    function new(); value=new(); endfunction
  endclass
  derived d;
  Box b;
  Header header_value;
  initial begin
    d=new(); b=new(); header_value=new();
    $display("H|%0d|%0d|%0d|%0d|%0d", d.get(), b.value.x, header_value.x, Wrap#(base)::get(), make_unit().x);
    $display("N|%s", $typename(b.value));
  end
endmodule
module top; worker u(); initial begin #1; $finish; end endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("H|7|7|5|7|5"), "{out}");
    assert!(
        !out.contains("worker::base"),
        "internal class key leaked: {out}"
    );
}

#[test]
fn classes_in_sibling_generate_scopes_have_separate_types() {
    let (ok, out) = run(
        r#"
class base; int x=5; endclass
module top;
  if (1) begin : a
    class base; int x=7; endclass
    function automatic int get(); base b=new(); return b.x; endfunction
  end
  if (1) begin : b
    class base; int x=9; endclass
    function automatic int get(); base v=new(); return v.x; endfunction
  end
  initial begin $display("G|%0d|%0d", a.get(), b.get()); $finish; end
endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("G|7|9"), "{out}");
}

#[test]
fn undeclared_class_type_actual_is_rejected() {
    for arg in ["wrong_wrong_wrong", ".T(wrong_wrong_wrong)"] {
        let src = format!(
            "class B; endclass class P #(int N=32,type T=B); T value; endclass class D extends P #({}); endclass module top; D d=new(); endmodule",
            if arg.starts_with('.') {
                arg.to_string()
            } else {
                format!("32,{arg}")
            }
        );
        let (ok, out) = run(&src, &[]);
        assert!(
            !ok && out.contains("wrong_wrong_wrong") && out.contains("not a declared type"),
            "{out}"
        );
    }
}

#[test]
fn class_type_defaults_are_optional_and_do_not_restrict_actual_subtypes() {
    let (ok, out) = run(
        r#"
class B; endclass
class Other; endclass
class P #(int N=32, type T=B); T value; endclass
class D extends P #(8,Other); endclass
module top;
  P #(8) p=new(); D d=new();
  initial begin $display("PASS"); $finish; end
endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("PASS"), "{out}");
    for argument in [
        "byte",
        "shortint",
        "int",
        "logic",
        "string",
        "real",
        "logic [7:0]",
    ] {
        let src = format!(
            "class P #(type T=int); endclass module top; P#({argument}) p=new(); initial $display(\"PASS\"); endmodule"
        );
        let (ok, out) = run(&src, &[]);
        assert!(ok && out.contains("PASS"), "{argument}: {out}");
    }
    let (ok, out) = run(
        r#"
package p; typedef struct packed {int x;} item_t; endpackage
import p::*;
class P #(type T=item_t); T value; endclass
module top; P#(item_t) value=new(); initial $display("PASS"); endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("PASS"), "{out}");
}

#[test]
fn illegal_event_operations_and_queue_methods_are_rejected() {
    for operation in [
        "e=new();",
        "e.trigger();",
        "e.is_triggered();",
        "e.clear();",
        "q.existing(0);",
        "q.exists(0);",
    ] {
        let src = format!(
            "module top; event e; int q[$]; initial begin {operation} $finish; end endmodule"
        );
        let (ok, out) = run(&src, &[]);
        assert!(
            !ok && (out.contains("not defined") || out.contains("cannot construct an event")),
            "{operation}: {out}"
        );
    }
}

#[test]
fn legal_events_queue_reductions_and_class_methods_stay_legal() {
    let (ok, out) = run(
        r#"
class C;
  function void trigger(); endfunction
  function void clear(); endfunction
endclass
module top;
  event e, alias_e;
  int q[$]='{3,1,2};
  C c=new();
  initial begin
    alias_e=e; -> e; q.sort(); c.trigger(); c.clear();
    $display("P|%0d|%0d|%0d", alias_e.triggered, q.sum(), q[0]);
    #1; $display("E|%0d", e.triggered); $finish;
  end
endmodule
"#,
        &[],
    );
    assert!(
        ok && out.contains("P|1|6|1") && out.contains("E|0"),
        "{out}"
    );
}

#[test]
fn parenthesized_member_select_is_rejected() {
    let (ok, out) = run(
        "module top; typedef struct packed {int a; int b;} S; S ps; initial $display(\"%0d\",(ps).b); endmodule",
        &[],
    );
    assert!(!ok && out.contains("parenthesized expression"), "{out}");
    let (ok, out) = run(
        "class C; function int get(); return 7; endfunction endclass module top; C c=new(); initial $display(\"M|%0d|%0d\",(c).get,(c).get()); endmodule",
        &[],
    );
    assert!(ok && out.contains("M|7|7"), "legal method root: {out}");
}

#[test]
fn named_top_preserves_generate_parameter_array_shapes() {
    let (ok, out) = run(
        r#"
module unused; endmodule
module top;
  if (1) begin : g
    localparam logic [31:0] L[2]='{32'h9,32'h3ff};
    localparam logic [1:0][7:0] P=16'habcd;
    initial begin $display("P|%h|%h|%h|%h",L[0],L[1],P[0],P[1]); $finish; end
  end
endmodule
"#,
        &[],
    );
    assert!(ok && out.contains("P|00000009|000003ff|cd|ab"), "{out}");
}
