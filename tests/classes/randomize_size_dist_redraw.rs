//! §18.5.4, §18.4: a `dist` on a dynamic array's or a queue's `size()` is drawn
//! by weight on every `randomize()`. The size solver had no form for it: a
//! fresh object took the joint solver's draw, and every later call kept the
//! length the array had, so 400 calls on one object gave one size 400 times.
//! The reference simulator gives 193:207 and 100:300 for this source.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  class D;
    rand bit [7:0] p[];
    rand bit [7:0] q[$];
    constraint s { p.size() dist {2 := 1, 6 := 1}; q.size() dist {1 :/ 1, [3:4] :/ 3}; }
  endclass
  initial begin
    D d = new;
    int p2 = 0, p6 = 0, q1 = 0, q34 = 0, other = 0;
    repeat (400) begin
      if (!d.randomize()) other++;
      if (d.p.size() == 2) p2++; else if (d.p.size() == 6) p6++; else other++;
      if (d.q.size() == 1) q1++; else if (d.q.size() inside {3, 4}) q34++; else other++;
    end
    $display("T| p2=%0d p6=%0d q1=%0d q34=%0d other=%0d", p2, p6, q1, q34, other);
  end
endmodule
"#;

#[test]
fn size_dist_is_redrawn_on_every_randomize() {
    let sim = simulate(SRC, 100).expect("simulate");
    let line = sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .find(|m| m.starts_with("T|"))
        .expect("result line");
    let get = |k: &str| -> i64 {
        line.split_whitespace()
            .find_map(|t| t.strip_prefix(k)?.strip_prefix('=')?.parse().ok())
            .unwrap_or_else(|| panic!("{k} missing from {line}"))
    };
    // About four standard deviations around 200:200 and 100:300.
    assert!((150..=250).contains(&get("p2")), "{line}");
    assert!((150..=250).contains(&get("p6")), "{line}");
    assert!((60..=140).contains(&get("q1")), "{line}");
    assert_eq!(get("q1") + get("q34"), 400, "{line}");
    assert_eq!(get("other"), 0, "{line}");
}
