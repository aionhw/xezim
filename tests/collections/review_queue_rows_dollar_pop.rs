//! IEEE 1800-2023 §7.10.1, §7.10.2.4, §7.10.2.5: review follow-ups for
//! queues of fixed-size arrays. A whole row read with `$` in its index
//! (`row = mq[$]`, `row = mq[$-1]`) reads the row `$` names; it read the row
//! of index x and copied nothing. `pop_front` and `pop_back`
//! of an object's queue of rows (`k.q.pop_front()`, `q.pop_front()` in a
//! method) return the removed row, as they do for a module queue. The
//! reference simulator prints the expected lines.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef int pair_t[2];
  class K;
    pair_t q[$];
    int r[$][2];
    function pair_t take(); return q.pop_front(); endfunction
  endclass
  K k;
  pair_t mq[$];
  pair_t row;
  int r2[2];
  initial begin
    k = new;
    mq.push_back('{5, 6}); mq.push_back('{7, 8});
    row = mq[$]; $display("T|row$|%p", row);
    row = mq[$-1]; $display("T|row$-1|%p", row);
    k.q.push_back('{1, 2}); k.q.push_back('{3, 4}); k.q.push_back('{5, 6}); k.q.push_back('{7, 8});
    row = k.q.pop_front(); $display("T|kfront|%p|%0d", row, k.q.size());
    $display("T|kback|%p|%0d", k.q.pop_back(), k.q.size());
    row = k.take(); $display("T|ktake|%p|%0d", row, k.q.size());
    k.r.push_back('{9, 10});
    r2 = k.r.pop_back(); $display("T|kr|%p|%0d", r2, k.r.size());
  end
endmodule
"#;

#[test]
fn dollar_row_reads_and_property_pops() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            r#"T|row$|'{7, 8}"#,
            r#"T|row$-1|'{5, 6}"#,
            r#"T|kfront|'{1, 2}|3"#,
            r#"T|kback|'{7, 8}|2"#,
            r#"T|ktake|'{3, 4}|1"#,
            r#"T|kr|'{9, 10}|0"#,
        ]
    );
}
