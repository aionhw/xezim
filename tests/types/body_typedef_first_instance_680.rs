//! IEEE 1800-2023 §6.18, §7.2.1, §13.4.3: a struct typedef declared in a
//! module body is seen by the constants of the module's first instance: a
//! constant function's struct formal, local and result, a struct
//! localparam's members (in constants and at run time), and a pattern
//! actual of a struct formal, also in a generate condition (#680, reported
//! by git-hevangel). Counts match the expected output.
use xezim::simulate;

fn counts(src: &str) -> (usize, usize) {
    let out: Vec<String> = simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .collect();
    let n = |p: &str| out.iter().filter(|m| m.starts_with(p)).count();
    (n("PASS"), n("FAIL"))
}

#[test]
fn constants_and_run_time_reads() {
    assert_eq!(
        counts(
            r#"// Self-checking: prints PASS or FAIL for each check, once per instance.
// A struct typedef declared in a module body, used by the constants of the
// instances of that module (IEEE 1800-2023 6.18, 7.2, 13.4.3).

// Every instance of `leaf` declares and uses its own struct type `cfg_t`.
module leaf #(parameter int W = 6);
  typedef struct packed { logic [W-1:0] a; logic [3:0] b; } cfg_t;

  // A constant function with a struct formal, a struct local variable and a
  // struct result.
  function automatic int pick_b(input cfg_t c);
    return c.b + 1;
  endfunction
  function automatic int sum_local(input int x);
    cfg_t t;
    t.a = x;
    t.b = x + 1;
    return t.a + t.b;
  endfunction
  function automatic cfg_t make(input int x);
    cfg_t r;
    r.a = x;
    r.b = x + 1;
    return r;
  endfunction

  localparam cfg_t K = '{a: 5, b: 7};
  localparam int FORMAL  = pick_b(K);                 // 8
  localparam int PATTERN = pick_b('{b: 7, a: 5});     // 8, names, not positions
  localparam int LOCAL   = sum_local(3);              // 3 + 4
  localparam cfg_t MADE  = make(3);
  localparam int MEMBER  = K.b;                       // 7
  localparam int MADE_B  = MADE.b;                    // 4

  task automatic check(input string what, input int got, input int want);
    if (got == want) $display("PASS %m %s = %0d", what, got);
    else             $display("FAIL %m %s = %0d, expected %0d", what, got, want);
  endtask

  initial begin
    check("pick_b(K)", FORMAL, 8);
    check("pick_b('{b: 7, a: 5})", PATTERN, 8);
    check("sum_local(3)", LOCAL, 7);
    check("make(3).b", MADE_B, 4);
    check("localparam K.b", MEMBER, 7);
    check("run-time K.b", int'(K.b), 7);
    check("run-time K.a", int'(K.a), 5);
  end
endmodule

// A second module that declares a type of the same name with the members in
// the other order: its instances use their own.
module swapped;
  typedef struct packed { logic [3:0] b; logic [5:0] a; } cfg_t;
  function automatic int pick_a(input cfg_t c);
    return c.a;
  endfunction
  localparam cfg_t K = '{b: 7, a: 5};
  localparam int FORMAL = pick_a(K);                  // 5
  localparam int MEMBER = K.a;                        // 5

  initial begin
    if (FORMAL == 5) $display("PASS %m pick_a(K) = %0d", FORMAL);
    else             $display("FAIL %m pick_a(K) = %0d, expected 5", FORMAL);
    if (MEMBER == 5) $display("PASS %m localparam K.a = %0d", MEMBER);
    else             $display("FAIL %m localparam K.a = %0d, expected 5", MEMBER);
  end
endmodule

module top;
  leaf u0 ();                // the first instance of `leaf`
  leaf u1 ();                // later instances
  leaf #(.W(9)) u2 ();
  swapped s0 ();
  swapped s1 ();
  initial #1 $finish;
endmodule
"#
        ),
        (25, 0)
    );
}

#[test]
fn pattern_actual_in_a_generate_condition() {
    assert_eq!(
        counts(
            r#"// A pattern written as the actual of a constant function in a generate
// condition (IEEE 1800-2023 13.4.3, 27.5): prints PASS or FAIL per module.
module leaf;
  typedef struct packed { logic [5:0] a; logic [3:0] b; } cfg_t;
  function automatic int pick(input cfg_t c);
    return c.b + 1;
  endfunction
  if (pick('{b: 4'd7, a: 6'd5}) == 8) begin : g_ok
    initial $display("PASS %m: pick('{b: 7, a: 5}) is 8");
  end else begin : g_bad
    initial $display("FAIL %m: the else branch is taken, pick('{b: 7, a: 5}) is not 8");
  end
endmodule

module top;
  typedef struct packed { logic [5:0] a; logic [3:0] b; } cfg_t;
  function automatic int pick(input cfg_t c);
    return c.b + 1;
  endfunction
  if (pick('{b: 4'd7, a: 6'd5}) == 8) begin : g_ok
    initial $display("PASS %m: pick('{b: 7, a: 5}) is 8");
  end else begin : g_bad
    initial $display("FAIL %m: the else branch is taken, pick('{b: 7, a: 5}) is not 8");
  end
  leaf u0 ();
  leaf u1 ();
endmodule
"#
        ),
        (3, 0)
    );
}
