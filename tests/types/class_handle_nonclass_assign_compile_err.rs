//! Compile-time rejection of an illegal §6.18 assignment: assigning a
//! non-class value (an enum member / integer literal) to a class-handle
//! variable is a compile error — "only a class handle or null is allowed".
//!
//! The reference elaborator rejects `comp = NONE;` (an enum member into a
//! class handle) at compile time. xezim runs the procedural body at time 0,
//! so without this check it silently coerced the enum and the run progressed.
//! `simulate()` must therefore return `Err` here, and legal class-handle
//! assignments (`= new`, `= null`, `= obj`) must still run.
use xezim::simulate;

#[test]
fn illegal_enum_to_class_handle_is_compile_error() {
    // `NONE` is an enum member; `comp` is a class handle. The reference
    // rejects this at compile time, and so must xezim.
    const BAD: &str = r#"
module top;
  class my_class;
    int x;
  endclass

  typedef enum { NONE = 0, ONE = 1 } verbosity_e;

  initial begin
    my_class comp;
    comp = NONE;
    $display("SHOULD NOT REACH");
  end
endmodule
"#;
    match simulate(BAD, 1) {
        Ok(_) => panic!("compiling enum→class-handle assignment must fail"),
        Err(e) => {
            assert!(
                e.contains("illegal assignment"),
                "unexpected error text: {}",
                e
            );
        }
    }
}

#[test]
fn legal_class_handle_assignments_still_compile() {
    // `obj = new`, `copy = null`, `b = a` are all legal; they must NOT be
    // rejected by the compile-time check.
    const GOOD: &str = r#"
module top;
  class my_class;
    int x;
    function new();
      x = 7;
    endfunction
  endclass

  initial begin
    my_class a;
    my_class b;
    my_class c;
    a = new;
    b = null;
    c = a;
    if (c.x == 7) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(GOOD, 1).expect("legal class assignments must compile");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("TAG_PASS"), "got:\n{}", out);
}