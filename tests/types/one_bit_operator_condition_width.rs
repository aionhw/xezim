//! §11.6.1, Table 11-21: an equality, relational, logical, reduction or
//! `inside` operator yields a 1-bit result; its operands are sized among
//! themselves and never widen the expression around it. A condition (`if`,
//! `?:`, loops, `case` items, `wait`, an event's `iff`, an assertion) is
//! self-determined (§12.4, §11.4.11), so in `(~a) | (m == 5'd12)` over a
//! 1-bit `a` the `|` is 1 bit and `~a` is 0. xezim's interpreter sized the
//! `|` at the compare's 5-bit operands, read `~a` as 5'b11110 and took the
//! branch; an assignment hid it by truncating to its 1-bit target.
//!
//! Every expectation matches Verilator and Icarus.

use xezim::simulate;

fn outs(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn assert_passes(o: &[String]) {
    assert!(
        !o.iter().any(|l| l.starts_with("FAIL")),
        "failing checks: {o:?}"
    );
    assert!(o.iter().any(|l| l == "PASS"), "no PASS line: {o:?}");
}

#[test]
fn interpreted_conditions_size_one_bit_operators_as_one_bit() {
    let src = r#"
package pk;
  typedef enum logic [4:0] { A_NONE, A_SKDEC, A_LFSR = 5'd12 } amode_e;
endpackage
module t;
  import pk::*;
  logic a = 1'b1, b = 1'b0;
  logic [4:0] m = 5'd1;
  amode_e e = A_SKDEC;
  int errs = 0;
  `define CHK(nm, got) if ((got) !== 1'b0) begin errs++; $display("FAIL %s got=%b", nm, got); end
  initial begin
    logic r; int n; logic [2:0] cc;
    #1;
    r = (~a) | (m == 5'd12);                                 `CHK("assign", r)
    if ((~a) | (m == 5'd12)) r = 1; else r = 0;             `CHK("if_eq", r)
    if ((~a) | (b & (e == A_LFSR))) r = 1; else r = 0;      `CHK("if_enum", r)
    r = ((~a) | (b & (m == 5'd12))) ? 1'b1 : 1'b0;           `CHK("cond_eq", r)
    if ((~a) | (m != 5'd1)) r = 1; else r = 0;              `CHK("if_neq", r)
    if ((~a) | (m === 5'd12)) r = 1; else r = 0;            `CHK("if_caseeq", r)
    if ((~a) | (m !== 5'd1)) r = 1; else r = 0;             `CHK("if_caseneq", r)
    if ((~a) | (m ==? 5'b11xx0)) r = 1; else r = 0;         `CHK("if_wildeq", r)
    if ((~a) | (m < 5'd1)) r = 1; else r = 0;               `CHK("if_lt", r)
    if ((~a) | (m >= 5'd9)) r = 1; else r = 0;              `CHK("if_ge", r)
    if ((~a) | (m[0] && m[1])) r = 1; else r = 0;           `CHK("if_land", r)
    if ((~a) | (m[1] || m[2])) r = 1; else r = 0;           `CHK("if_lor", r)
    if ((~a) | (!m)) r = 1; else r = 0;                     `CHK("if_lnot", r)
    if ((~a) | (&m)) r = 1; else r = 0;                     `CHK("if_redand", r)
    if ((~a) | (~|m[1:0])) r = 1; else r = 0;               `CHK("if_rednor", r)
    if ((~a) | (m inside {5'd12, [5'd20:5'd30]})) r = 1; else r = 0; `CHK("if_inside", r)
    if ((~a) | (b <-> m[0])) r = 1; else r = 0;             `CHK("if_equiv", r)
    if ((~a) ^ (m == 5'd12)) r = 1; else r = 0;             `CHK("if_xor", r)
    if (~((~a) | (m == 5'd1))) r = 1; else r = 0;           `CHK("if_not", r)
    n = 0; while ((~a) | (m == 5'd12)) begin n++; if (n > 3) break; end `CHK("while", n != 0)
    n = 0; for (int i = 0; (~a) | (m == 5'd12); i++) begin n++; if (n > 3) break; end `CHK("for", n != 0)
    n = 0; do begin n++; if (n > 3) break; end while ((~a) | (m == 5'd12)); `CHK("do", n != 1)
    case (1'b1) ((~a) | (m == 5'd12)): r = 1; default: r = 0; endcase `CHK("case_item", r)
    r = 0; assert ((~a) | (m == 5'd12)) r = 1; else r = 0;   `CHK("assert", r)
    cc = {(~a) | (m == 5'd12), 2'b11};                      `CHK("concat", cc != 3'b011)
    `CHK("bits", $bits((~a) | (m == 5'd12)) != 1)
    $display("disp=%b", (~a) | (m == 5'd12));
    if (errs == 0) $display("PASS");
  end
endmodule
"#;
    let o = outs(src, 10);
    assert_passes(&o);
    assert!(o.iter().any(|l| l == "disp=0"), "{o:?}");
}

#[test]
fn compiled_and_scheduled_conditions_size_one_bit_operators_as_one_bit() {
    let src = r#"
module t;
  logic a = 1'b1, clk = 0;
  logic [4:0] m = 5'd1;
  int errs = 0;
  `define CHK(nm, got) if ((got) !== 1'b0) begin errs++; $display("FAIL %s got=%b", nm, got); end
  logic k1, k2, k3, k4, k5, k6, kf;
  assign k1 = ((~a) | (m == 5'd12)) ? 1'b1 : 1'b0;
  always_comb begin
    if ((~a) | (m == 5'd12)) k2 = 1; else k2 = 0;
    if ((~a) | (m ==? 5'b11xx0)) k3 = 1; else k3 = 0;
    if ((~a) | (~|m[1:0])) k4 = 1; else k4 = 0;
    case (1'b1) ((~a) | (m == 5'd12)): k5 = 1; default: k5 = 0; endcase
  end
  always_ff @(posedge clk) k6 <= ((~a) | (m == 5'd12)) ? 1'b1 : 1'b0;
  function automatic logic f1(logic x, logic [4:0] y);
    if ((~x) | (y == 5'd12)) return 1; else return 0;
  endfunction
  assign kf = f1(a, m);
  logic w_fired = 0, e_fired = 0;
  initial fork
    begin wait ((~a) | (m == 5'd12)); w_fired = 1; end
    begin @(posedge clk iff ((~a) | (m == 5'd12))); e_fired = 1; end
  join_none
  always #5 clk = ~clk;
  int ap_fail = 0, cv_hits = 0;
  ap: assert property (@(posedge clk) !((~a) | (m == 5'd12))) else ap_fail++;
  cv: cover property (@(posedge clk) (~a) | (m == 5'd12)) cv_hits++;
  initial begin
    #40;
    `CHK("assign", k1) `CHK("comb_if", k2) `CHK("comb_wildeq", k3)
    `CHK("comb_rednor", k4) `CHK("comb_case", k5) `CHK("ff", k6) `CHK("func", kf)
    `CHK("wait", w_fired) `CHK("event_iff", e_fired)
    `CHK("assert_property", ap_fail != 0) `CHK("cover_property", cv_hits != 0)
    if (errs == 0) $display("PASS");
    $finish;
  end
endmodule
"#;
    assert_passes(&outs(src, 100));
}
