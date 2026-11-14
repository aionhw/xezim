//! IEEE 1800-2023 §9.4.5: an intra-assignment timing control evaluates its
//! right-hand side once, when the statement runs. A pattern for a packed
//! target was evaluated twice, so each function call in it ran twice, and a
//! conditional of unpacked patterns (§11.4.11) stored nothing after the
//! delay. The values were checked on the reference simulator.
use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let result = simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn a_packed_pattern_is_evaluated_once() {
    let lines = run(r#"
module t;
  typedef struct packed { logic [7:0] x; logic [7:0] y; } ps_t;
  int cnt = 0;
  function automatic int f(); cnt++; return cnt; endfunction
  ps_t ps, pe;
  event e;
  initial begin
    ps = #1 '{f(), f()};
    $display("R ps=%0d %0d cnt=%0d t=%0t", ps.x, ps.y, cnt, $time);
    cnt = 0;
    fork
      pe = @(e) '{f(), f()};
      #1 -> e;
    join
    $display("R pe=%0d %0d cnt=%0d", pe.x, pe.y, cnt);
  end
endmodule
"#);
    assert_eq!(lines, ["R ps=1 2 cnt=2 t=1", "R pe=1 2 cnt=2"]);
}

#[test]
fn a_conditional_of_unpacked_patterns_selects_its_operand_now() {
    let lines = run(r#"
module t;
  int wa [2], wb [2], wc [2];
  bit c = 1;
  event e;
  initial begin
    fork
      wa = #2 c ? '{1, 2} : '{3, 4};
      #1 c = 0;
    join
    wb = #1 c ? '{1, 2} : '{3, 4};
    fork
      wc = @(e) !c ? '{5, 6} : '{7, 8};
      #1 -> e;
    join
    $display("R wa=%0d %0d wb=%0d %0d wc=%0d %0d", wa[0], wa[1], wb[0], wb[1], wc[0], wc[1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R wa=1 2 wb=3 4 wc=5 6"]);
}
