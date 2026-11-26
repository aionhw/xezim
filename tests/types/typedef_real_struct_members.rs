//! Unpacked struct members retain their real type through typedefs (§6.18,
//! §7.2) and class type parameters (§8.25).

use xezim::simulate;

fn check_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 20).expect("simulate failed");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.trim().strip_prefix("CHECK "))
        .collect();
    assert_eq!(actual, expected);
}

/// §6.6.7: the resolver's result is the net value. The fixed array in its
/// local result has real elements through the alias (§6.18, §7.2, §7.4.2).
#[test]
fn resolver_returns_typedef_real_array_members() {
    let src = r#"
module top;
  typedef real R;
  typedef struct { R m[2]; real b; } S;
  function automatic S resolve(input S drivers[]);
    S r;
    r.m[0] = drivers[0].m[0];
    r.m[1] = drivers[0].m[1];
    r.b = drivers[0].b;
    return r;
  endfunction
  nettype S NR with resolve;
  NR n;
  assign n = '{m:'{1.5, 2.5}, b:3.5};
  initial begin
    #1;
    $display("CHECK %0.1f %0.1f %0.1f", n.m[0], n.m[1], n.b);
  end
endmodule
"#;
    check_output(src, &["1.5 2.5 3.5"]);
}

/// §§6.12, 6.18, 7.2 and 7.4.4: local members retain the underlying real
/// type across alias chains, package qualification and nested arrays.
/// §6.12.1: assignment to the integer elements still rounds.
#[test]
fn function_local_members_keep_their_element_types() {
    let src = r#"
package p;
  typedef real R;
endpackage

module top;
  typedef int R;
  typedef p::R Alias;
  typedef shortreal SR;
  typedef realtime RT;
  typedef int I;
  typedef struct { Alias m[2]; } Inner;
  typedef struct {
    p::R m[2];
    SR s[2];
    RT t[2][2];
    Inner n;
    Alias scalar;
    real direct[2];
    I integral[2];
  } S;

  function automatic void check_members();
    S r;
    r.m[0] = -1.5;
    r.m[1] = 2.5;
    r.s[0] = 1.5;
    r.s[1] = 2.5;
    r.t[0][1] = 1.5;
    r.t[1][0] = 2.5;
    r.n.m[0] = 1.5;
    r.n.m[1] = 2.5;
    r.scalar = 1.5;
    r.direct[0] = 1.5;
    r.direct[1] = 2.5;
    r.integral[0] = 1.5;
    r.integral[1] = 2.5;
    $display("CHECK package %0.1f %0.1f", r.m[0], r.m[1]);
    $display("CHECK shortreal %0.1f %0.1f", r.s[0], r.s[1]);
    $display("CHECK realtime %0.1f %0.1f", r.t[0][1], r.t[1][0]);
    $display("CHECK nested %0.1f %0.1f", r.n.m[0], r.n.m[1]);
    $display("CHECK scalar %0.1f", r.scalar);
    $display("CHECK direct %0.1f %0.1f", r.direct[0], r.direct[1]);
    $display("CHECK integral %0d %0d", r.integral[0], r.integral[1]);
  endfunction

  initial begin
    #1;
    check_members();
  end
endmodule
"#;
    check_output(
        src,
        &[
            "package -1.5 2.5",
            "shortreal 1.5 2.5",
            "realtime 1.5 2.5",
            "nested 1.5 2.5",
            "scalar 1.5",
            "direct 1.5 2.5",
            "integral 2 3",
        ],
    );
}

/// §7.2.2: declared member initializers are assignments to the member's
/// type. §6.18 preserves that type in a task local and a class property.
#[test]
fn declared_scalar_member_initializers_use_the_underlying_type() {
    let src = r#"
module top;
  typedef real R;
  typedef int I;
  typedef struct { R a = 1.5; real b = 2.5; I i = 1.5; } S;
  class C;
    S s;
  endclass

  task automatic check_local();
    S r;
    $display("CHECK task %0.1f %0.1f %0d", r.a, r.b, r.i);
  endtask

  C obj;
  initial begin
    obj = new;
    #1;
    check_local();
    $display("CHECK class %0.1f %0.1f %0d", obj.s.a, obj.s.b, obj.s.i);
  end
endmodule
"#;
    check_output(src, &["task 1.5 2.5 2", "class 1.5 2.5 2"]);
}

/// §§6.18, 8.25 and 23.9: the class type parameters shadow the outer
/// real typedefs. Under §11.4.3 the division truncates for integral
/// members and keeps the fraction for real members. The integer input
/// rounds 1.5 to 2 on copy-in (§6.12.1, §13.5.1).
#[test]
fn class_type_parameters_shadow_outer_real_typedefs() {
    let src = r#"
typedef real UnitT;

module top;
  typedef real ModuleT;
  class C #(type UnitT = int, type ModuleT = int);
    typedef struct {
      UnitT unit_array[2];
      UnitT unit_scalar;
      ModuleT module_array[2];
      ModuleT module_scalar;
    } S;

    function void check_members(string label);
      S r;
      r.unit_array[0] = 3;
      r.unit_array[1] = 5;
      r.unit_scalar = 7;
      r.module_array[0] = 3;
      r.module_array[1] = 5;
      r.module_scalar = 7;
      $display("CHECK %s unit %0.1f %0.1f %0.1f", label,
               r.unit_array[0] / 2, r.unit_array[1] / 2, r.unit_scalar / 2);
      $display("CHECK %s module %0.1f %0.1f %0.1f", label,
               r.module_array[0] / 2, r.module_array[1] / 2, r.module_scalar / 2);
    endfunction

    function ModuleT twice(input UnitT i);
      UnitT local_value;
      local_value = i;
      twice = local_value * 2;
    endfunction
  endclass

  C #(int, int) ints;
  C #(real, real) reals;
  initial begin
    ints = new;
    reals = new;
    #1;
    ints.check_members("int");
    reals.check_members("real");
    $display("CHECK int call %0d", ints.twice(1.5));
  end
endmodule
"#;
    check_output(
        src,
        &[
            "int unit 1.0 2.0 3.0",
            "int module 1.0 2.0 3.0",
            "real unit 1.5 2.5 3.5",
            "real module 1.5 2.5 3.5",
            "int call 4",
        ],
    );
}
