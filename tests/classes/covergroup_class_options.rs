//! §19.7 instance options of a covergroup declared in a class (§19.4).
//! Inside the class, `cc.option.<name>` and `this.cc.option.<name>` parse
//! as member accesses; through a handle, `o.cc.option.<name>` is a
//! hierarchical name of four segments. Both used to miss the option
//! access: reads inside the class gave 0 and reads through a handle gave
//! `x`, even for an option the body sets to a literal.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn assert_line(lines: &[String], want: &str) {
    assert!(
        lines.iter().any(|l| l == want),
        "expected `{want}`, got {lines:?}"
    );
}

const CLASS: &str = r#"
  class C;
    bit [1:0] x;
    covergroup cc;
      option.goal = 70;
      option.per_instance = 1;
      cp: coverpoint x;
    endgroup
    function new; cc = new; endfunction
    function void show(string tag);
      $display("%s in %0d this %0d pi %0d al %0d w %0d", tag, cc.option.goal,
               this.cc.option.goal, cc.option.per_instance, cc.option.at_least,
               cc.option.weight);
    endfunction
    function void set_weight(int w); cc.option.weight = w; endfunction
  endclass
"#;

/// Reads inside the class: the body's literals, then a default.
#[test]
fn options_read_inside_the_class() {
    let l = lines(&format!(
        r#"
module tb;
{CLASS}
  C o;
  initial begin
    o = new;
    o.show("a");
  end
endmodule
"#
    ));
    assert_line(&l, "a in 70 this 70 pi 1 al 1 w 1");
}

/// Reads through a handle, and through a handle held by another object.
#[test]
fn options_read_through_a_handle() {
    let l = lines(&format!(
        r#"
module tb;
{CLASS}
  class W; C c; function new; c = new; endfunction endclass
  C o;
  W w;
  initial begin
    o = new;
    w = new;
    $display("h %0d %0d %0d", o.cc.option.goal, o.cc.option.at_least, w.c.cc.option.goal);
  end
endmodule
"#
    ));
    assert_line(&l, "h 70 1 70");
}

/// Writes inside the class and through a handle are read back by both.
#[test]
fn options_written_inside_the_class_and_through_a_handle() {
    let l = lines(&format!(
        r#"
module tb;
{CLASS}
  C o;
  initial begin
    o = new;
    o.set_weight(3);
    o.cc.option.goal = 55;
    o.show("b");
    $display("h %0d %0d", o.cc.option.weight, o.cc.option.goal);
  end
endmodule
"#
    ));
    assert_line(&l, "b in 55 this 55 pi 1 al 1 w 3");
    assert_line(&l, "h 3 55");
}

/// Guard: a struct member and a class property that are themselves called
/// `option` keep their own members, and a module-level covergroup's option
/// reads as before.
#[test]
fn members_called_option_and_module_covergroups_are_unchanged() {
    let l = lines(
        r#"
module tb;
  typedef struct { int x; } in_t;
  typedef struct { in_t option; } out_t;
  class K; int x; endclass
  class H; K option; function new; option = new; option.x = 8; endfunction
    function int get; return option.x; endfunction endclass
  bit [1:0] v;
  covergroup cg; option.goal = 60; cp: coverpoint v; endgroup
  out_t s;
  H h;
  cg c = new;
  initial begin
    s.option.x = 5;
    h = new;
    $display("g %0d %0d %0d %0d", s.option.x, h.option.x, h.get(), c.option.goal);
  end
endmodule
"#,
    );
    assert_line(&l, "g 5 8 8 60");
}
