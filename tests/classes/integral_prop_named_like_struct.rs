//! A class property that is a scalar INTEGER (not a struct) assigned in a
//! method that a same-named UNPACKED struct elsewhere shadows.
//!
//! `var_decl_types` is keyed by the bare name and shared by every scope
//! (#258), so an unrelated module-scope unpacked struct variable `s` makes
//! `struct_copy_target("s")` answer a struct. UVM's field-automation macros
//! assign `s = __rdone__` for an `int s` (e.g. the 3497 autocfgprec test)
//! while some other scope declares a struct `s`: the whole-struct copy branch
//! diverted the write to the integral property and left `s` stuck at 0.
//!
//! (The class-handle case is covered by `class_formal_named_like_struct`; this
//! is the integral-property variant fixed alongside it.)

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
typedef struct { bit [7:0] a; } s_t;
"#;

/// A module-scope unpacked struct variable `s` must not divert `s = v` inside
/// a method writing `this.s` (an `int` property).
#[test]
fn integral_property_write_not_diverted_by_module_struct() {
    let src = format!(
        "{COMMON}{}",
        r#"
class cls;
  int s;
  function void set_s(int val);
    s = val;
  endfunction
endclass

module top;
  s_t s; // unrelated unpacked struct variable sharing the bare name
  initial begin
    automatic cls c = new;
    c.s = 0;
    c.set_s(9);
    if (c.s == 9) $display("TAG_PASS");
    else $display("TAG_FAIL s=%0d", c.s);
  end
endmodule
"#
    );
    assert_eq!(messages(&src), ["TAG_PASS"]);
}

/// The same write through a bare-name assignment produces the exact value the
/// UVM field-op uses (`s = __rdone__`), exercised through several small
/// integers to make sure nothing lands in the struct's leaf instead.
#[test]
fn integral_property_assign_survives_struct_shadow() {
    let src = format!(
        "{COMMON}{}",
        r#"
class cls;
  int s;
  int v;
  function void set(int a, int b);
    v = a;
    s = b;
  endfunction
endclass

module top;
  s_t s;
  initial begin
    automatic cls c = new;
    int i;
    for (i = 0; i < 3; i++) begin
      c.set(i, 10 + i);
      if (c.s != 10 + i || c.v != i)
        $display("TAG_FAIL i=%0d s=%0d v=%0d", i, c.s, c.v);
    end
    if (c.s == 12 && c.v == 2) $display("TAG_PASS");
    else $display("TAG_FAIL s=%0d v=%0d", c.s, c.v);
  end
endmodule
"#
    );
    assert_eq!(messages(&src), ["TAG_PASS"]);
}