//! §10.9.1, §7.10.2.2, §10.10: patterns and concatenations pushed onto a
//! queue of queues. A replicated pattern (`'{3{7}}`, `'{2{1, 2}}`) is that
//! many elements, so the row gets all of them and grows from there; byte and
//! string rows take their items fitted to the element type (#1086 review).
//! Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn replicated_patterns_pushed_into_queues_of_queues() {
    assert_eq!(
        lines(
            r#"module qq5;
  int qq[$][$];
  byte qb[$][$];
  string qs[$][$];
  int tmp[$];
  initial begin
    qq.push_back('{3{7}});
    qq.push_back({});
    qq.push_back('{9});
    qq.push_back('{2{1, 2}});
    $display("A %0d | %0d %0d %0d %0d | %0d %0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[2].size(), qq[3].size(), qq[0][2], qq[2][0], qq[3][2]);
    tmp = {1, 2, 3};
    qq.push_back({tmp, 4});
    qq[0].push_back(8);
    $display("B %0d %0d %0d %0d", qq[4].size(), qq[4][3], qq[0].size(), qq[0][3]);
    qb.push_back({300, -1, 8'h7f});
    $display("C %0d %0d %0d %0d", qb[0].size(), qb[0][0], qb[0][1], qb[0][2]);
    qs.push_back({"ab", "c"});
    qs.push_back('{"x", "yz", ""});
    $display("D %0d %0d %s %s %0d %s", qs[0].size(), qs[1].size(), qs[0][0], qs[0][1], qs[1][2].len(), qs[1][1]);
  end
endmodule
"#
        ),
        [
            r#"A 4 | 3 0 1 4 | 7 9 1"#,
            r#"B 4 4 4 8"#,
            r#"C 3 44 -1 127"#,
            r#"D 2 3 ab c 0 yz"#,
        ]
    );
}
