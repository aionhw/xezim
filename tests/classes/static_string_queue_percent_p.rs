//! IEEE 1800-2023 §21.2.1.7, §8.9: `%p` of a static queue of a class prints
//! its elements as the declared element type, quoted strings for a
//! `string` queue, however it is named.

#[test]
fn static_string_queue_prints_strings() {
    let sim = xezim::simulate(
        r#"
module top;
  class C;
    static string sq[$];
    static int nq[$];
    static string pq[$];
    static function void show(); $display("T|in %p", sq); endfunction
  endclass
  C c;
  initial begin
    C::sq = {"a", "b"};
    C::nq = {1, 2};
    C::pq.push_back("p");
    c = new;
    $display("T|%p %p %p", C::sq, C::nq, C::pq);
    $display("T|%p", c.sq);
    C::show();
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            r#"T|'{"a", "b"} '{1, 2} '{"p"}"#,
            r#"T|'{"a", "b"}"#,
            r#"T|in '{"a", "b"}"#,
        ]
    );
}
