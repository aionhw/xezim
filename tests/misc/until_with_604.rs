//! IEEE 1800-2017 §16.12.12: `p until_with q` is `p until (p and q)`, and
//! `s_until_with` likewise from `s_until`. Parenthesized, `until_with` was a
//! parse error (#604, reported by amal-khailtash); without parentheses it
//! was skipped.
use xezim::simulate;

#[test]
fn reported_property_parses() {
    assert!(
        simulate(
            r#"module top;
  bit clk, a, b;
  property hold_until_done;
    @(posedge clk) a |-> (!a until_with b);
  endproperty
  assert property (hold_until_done);
endmodule

"#,
            10
        )
        .is_ok()
    );
}

/// `a` falls in the cycle `b` rises (passes `until`, fails `until_with`),
/// then holds through it (passes both); `c` never rises for `s_until_with`.
#[test]
fn until_with_requires_the_left_operand_in_the_final_cycle() {
    let src = r#"module top;
  bit clk, go, a, b, c;
  int u_ok, u_bad, w_ok, w_bad, s_ok, s_bad, n_ok, n_bad;
  always #5 clk = ~clk;
  a_u: assert property (@(posedge clk) go |-> (a until b)) u_ok++; else u_bad++;
  a_w: assert property (@(posedge clk) go |-> (a until_with b)) w_ok++; else w_bad++;
  a_n: assert property (@(posedge clk) go |-> a until_with b) n_ok++; else n_bad++;
  a_s: assert property (@(posedge clk) go |-> (a s_until_with c)) s_ok++; else s_bad++;
  initial begin
    // case 1: a holds, then drops in the cycle b rises
    @(negedge clk) go = 1; a = 1; b = 0;
    @(negedge clk) go = 0; a = 1;
    @(negedge clk) a = 0; b = 1;
    @(negedge clk) b = 0;
    // case 2: a holds through the cycle b rises
    @(negedge clk) go = 1; a = 1;
    @(negedge clk) go = 0;
    @(negedge clk) b = 1;
    @(negedge clk) b = 0; a = 0;
    repeat (3) @(negedge clk);
    $display("R u=%0d/%0d w=%0d/%0d n=%0d/%0d s=%0d/%0d", u_ok, u_bad, w_ok, w_bad, n_ok, n_bad, s_ok, s_bad);
    $finish;
  end
endmodule
"#;
    let out: Vec<String> = simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("R "))
        .collect();
    assert_eq!(out, ["R u=2/0 w=1/1 n=1/1 s=0/2"]);
}
