//! §18.12: the scope randomize function may be called without the `std::`
//! prefix (`scope_randomize ::= [ std:: ] randomize ( … ) [ with … ]`).
//! Outside a class, `randomize(x) with { x == 9; }` is `std::randomize`: it
//! solves the listed variables and returns 1, or returns 0 and leaves them
//! unchanged. Inside a class method a bare call is still `this.randomize`
//! (§18.11). Expected values follow the LRM and match `std::randomize`.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn scope_randomize_with_and_without_constraints() {
    let lines = run(r#"
module tb;
  bit [7:0] x; int ok;
  initial begin
    ok = randomize(x) with { x == 9; };
    $display("R with ok=%0d x=%0d", ok, x);
    ok = randomize(x);
    $display("R plain ok=%0d", ok);
  end
endmodule
"#);
    assert_eq!(lines, ["R with ok=1 x=9", "R plain ok=1"]);
}

#[test]
fn several_variables_and_set_membership() {
    let lines = run(r#"
module tb;
  bit [7:0] x, z; int y, ok, ok2;
  initial begin
    ok = randomize(x, z) with { x == 3; z == x + 1; };
    ok2 = randomize(y) with { y inside {[20:22]}; };
    $display("R ok=%0d x=%0d z=%0d ok2=%0d y_in=%0d", ok, x, z, ok2, y >= 20 && y <= 22);
  end
endmodule
"#);
    assert_eq!(lines, ["R ok=1 x=3 z=4 ok2=1 y_in=1"]);
}

#[test]
fn call_as_condition_void_cast_and_in_subroutines() {
    let lines = run(r#"
module tb;
  bit [7:0] x; int ok;
  task automatic tk();
    int l;
    ok = randomize(l) with { l == 4; };
    $display("R task ok=%0d l=%0d", ok, l);
  endtask
  function automatic int fn();
    ok = randomize(x) with { x == 6; };
    return x;
  endfunction
  initial begin
    if (randomize(x) with { x == 7; }) $display("R cond x=%0d", x);
    else $display("R cond failed");
    void'(randomize(x) with { x == 8; });
    $display("R void x=%0d", x);
    tk();
    $display("R fn=%0d ok=%0d", fn(), ok);
  end
endmodule
"#);
    assert_eq!(
        lines,
        ["R cond x=7", "R void x=8", "R task ok=1 l=4", "R fn=6 ok=1"]
    );
}

#[test]
fn unsatisfiable_call_returns_zero_and_keeps_the_value() {
    let lines = run(r#"
module tb;
  bit [7:0] x; int ok;
  initial begin
    x = 5;
    ok = randomize(x) with { x > 10; x < 5; };
    $display("R ok=%0d x=%0d", ok, x);
  end
endmodule
"#);
    assert_eq!(lines, ["R ok=0 x=5"]);
}

#[test]
fn empty_variable_list_checks_the_constraints() {
    let lines = run(r#"
module tb;
  bit [7:0] x; int a, b;
  initial begin
    x = 9;
    a = randomize() with { x == 9; };
    b = randomize() with { x == 1; };
    $display("R a=%0d b=%0d x=%0d", a, b, x);
  end
endmodule
"#);
    assert_eq!(lines, ["R a=1 b=0 x=9"]);
}

/// Guards: `std::randomize` and a bare call in a class method
/// (`this.randomize`) are unchanged.
#[test]
fn std_prefix_and_class_method_guards() {
    let lines = run(r#"
class C;
  rand bit [7:0] v;
  function int go();
    return randomize(v) with { v == 12; };
  endfunction
endclass
module tb;
  bit [7:0] x; int ok;
  C c;
  initial begin
    ok = std::randomize(x) with { x == 9; };
    $display("R std ok=%0d x=%0d", ok, x);
    c = new;
    ok = c.go();
    $display("R class ok=%0d v=%0d", ok, c.v);
  end
endmodule
"#);
    assert_eq!(lines, ["R std ok=1 x=9", "R class ok=1 v=12"]);
}
