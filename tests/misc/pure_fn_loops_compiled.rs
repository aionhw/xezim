//! Issue #146: the purity walker had no arms for `while`/`do-while`/
//! `repeat`/`break`/`continue`, so a loop-based pure helper was branded
//! `Expr_Call_impure` — and the bytecode compiler had no `While`/`DoWhile`
//! arms at all, so fixing purity alone would only have moved the bail.
//! Both landed together; every value below is reference-simulator verified.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn while_and_do_while_helpers_inline_and_compute() {
    let out = msgs(
        r#"
module top;
  function automatic int popcnt(input int x);
    int n; int v;
    n = 0; v = x;
    while (v != 0) begin n = n + (v & 1); v = v >> 1; end
    return n;
  endfunction
  function automatic int firstset(input int x);
    int i; int v;
    i = 0; v = x;
    do begin
      if (v & 1) break;
      v = v >> 1; i = i + 1;
      if (i > 31) begin i = -1; break; end
    end while (v != 0);
    return (v == 0 && i > 31) ? -1 : i;
  endfunction
  logic [31:0] a, y, z;
  assign y = popcnt(a);
  assign z = firstset(a);
  initial begin
    a = 32'h0;         #1 $display("Q1_%0d_%0d", y, z);
    a = 32'h0F00;      #1 $display("Q2_%0d_%0d", y, z);
    a = 32'h8000_0000; #1 $display("Q3_%0d_%0d", y, z);
    a = 32'h0F0F;      #1 $display("Q4_%0d", y);
  end
endmodule
"#,
    );
    assert!(out.contains(&"Q1_0_1".to_string()), "{out:?}");
    assert!(out.contains(&"Q2_4_8".to_string()), "{out:?}");
    assert!(out.contains(&"Q3_1_31".to_string()), "{out:?}");
    assert!(out.contains(&"Q4_8".to_string()), "{out:?}");
}
