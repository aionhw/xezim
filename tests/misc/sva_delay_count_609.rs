//! IEEE 1800-2017 §16.7: `cycle_delay_range ::= ## constant_primary`. A
//! name after `##` is the count even when `(` follows: `go ##N (x != y)[*2]`
//! is a delay of `N` then the parenthesized sequence, not a call `N(...)`
//! (#609, reported by git-hevangel). xezim also accepts `##pk::PN`
//! without parentheses.
use xezim::simulate;

fn shown(src: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| f(m))
        .collect()
}

#[test]
fn reported_testbench() {
    let out = shown(
        r#"// IEEE 1800-2017 16.7: cycle_delay_range ::= ## constant_primary | ...
// After ## exactly one constant primary is the delay count; what follows it
// starts the next sequence. A parameter name is such a primary, so in
//
//     go ##N (x != y)[*(M-N)] ##1 (x == y)
//
// the count is N, the parenthesised (x != y) is the operand of the repetition,
// and the sequence ends M ticks after `go`. It is not the call `N(x != y)`.
//
// One `go` pulse at a known tick; x != y holds at ticks N .. M-1 after it and
// x == y at tick M. The cover must match once, at tick M after `go`.
// Every line should print PASS. With the bug the file does not compile.
module t;
  parameter int N = 1;
  parameter int M = 3;

  logic clk = 0;
  logic go = 0, x = 0, y = 0;
  int   hits = 0;
  time  hit_time;
  int   fails = 0;
  time  go_time;

  always #5 clk = ~clk;                 // posedges at 5, 15, 25, ...

  cover property (@(posedge clk) go ##N (x != y)[*(M-N)] ##1 (x == y))
    begin hits++; hit_time = $time; end

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    // drive on negedges; `go` is high at the posedge at time 25 only
    @(negedge clk);                     // 10
    @(negedge clk);                     // 20
    go = 1; go_time = 25;
    @(negedge clk);                     // 30
    go = 0;
    // ticks after `go`: 1 .. N-1 are don't-care; x != y at N .. M-1; x == y at M
    for (int k = 1; k <= M; k++) begin
      x = 0; y = (k >= N && k < M);     // x != y while N <= k < M, equal at k == M
      @(negedge clk);
    end
    repeat (4) @(negedge clk);

    check("cover matched once",                 hits == 1);
    check("match ends M ticks after go",        hit_time == go_time + 10 * M);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

/// `##N (x)`, `##N(x)`, `##pk::PN (x)` and `##(f(2)) (x)` each match once.
#[test]
fn count_spellings() {
    assert_eq!(
        shown(
            r#"package pk; parameter int PN = 2; endpackage
module t;
  parameter int N = 2;
  logic clk = 0, go = 0, one = 0;
  int h1, h2, h3, h4;
  function int f(int k); return k; endfunction
  always #5 clk = ~clk;
  c1: cover property (@(posedge clk) go ##N (one)) h1++;
  c2: cover property (@(posedge clk) go ##N(one)) h2++;
  c3: cover property (@(posedge clk) go ##pk::PN (one)) h3++;
  c4: cover property (@(posedge clk) go ##(f(2)) (one)) h4++;
  initial begin
    @(negedge clk) go = 1; @(negedge clk) go = 0; @(negedge clk) one = 1; @(negedge clk) one = 0;
    repeat (3) @(negedge clk);
    $display("R %0d %0d %0d %0d", h1, h2, h3, h4); $finish;
  end
endmodule
"#,
            |m| m.starts_with("R ")
        ),
        ["R 1 1 1 1"]
    );
}
