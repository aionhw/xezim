//! §7.2.1, §6.8: a packed struct whose members are all 2-state is a 2-state
//! vector and starts at 0, also when a member is a packed array of another
//! 2-state struct (`s_t [1:0] e;`), whose type is reached through a typedef.
//! Such a variable started as x, while its 2-state writes
//! already dropped x/z across the whole value.
//!
//! Verilator 5.046 prints the same zeros, though it models no x; the 4-state
//! counterpart checks that a struct with a `logic` member still starts as x.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn assert_has(got: &[String], want: &str) {
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

#[test]
fn two_state_struct_with_a_struct_array_member_starts_at_zero() {
    let got = lines(
        r#"
module top;
  typedef struct packed { bit [7:0] a; bit [7:0] b; } s_t;
  typedef struct packed { s_t [1:0] e; } n_t;
  typedef struct packed { n_t [1:0] n; bit f; } nn_t;
  typedef struct packed { s_t [1:0] e; logic [7:0] l; } m_t;
  s_t s;
  n_t n;
  nn_t nn;
  m_t m;
  bit [1:0][7:0] pa;
  initial #1 $display("ti: s=%h n=%h nn=%h m=%h pa=%h", s, n, nn, m, pa);
endmodule
"#,
    );
    assert_has(&got, "ti: s=0000 n=00000000 nn=00000000000000000 m=xxxxxxxxxx pa=0000");
}

#[test]
fn unwritten_and_written_two_state_structs_agree() {
    let got = lines(
        r#"
module top;
  typedef struct packed { bit [7:0] value; } bdw_t;
  typedef struct packed { bdw_t [1:0] dw; } bnest_t;
  typedef struct packed { bnest_t [1:0] e; } bnest2_t;
  bnest2_t b, c;
  always_comb c.e[1].dw[0].value = 8'h05;
  initial #1 $display("ti3: b=%h c=%h", b, c);
endmodule
"#,
    );
    assert_has(&got, "ti3: b=00000000 c=00050000");
}
