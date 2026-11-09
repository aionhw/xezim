//! IEEE 1800-2023 §10.9.1, §10.10, §7.6, §8.4: follow-ups to assignment
//! patterns on rows of multi-dimensional unpacked arrays (#999). Expected
//! values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_lines(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(
            msgs.iter().any(|m| m.trim() == *w),
            "missing {w:?} in {msgs:#?}"
        );
    }
}

/// §8.4: in a method, `c[1] = '{7, 8}` names the property `c` of `this`,
/// not a same-named module array.
#[test]
fn row_pattern_in_a_method_stores_the_property() {
    let msgs = messages(
        r#"module t;
  class C;
    int c [2][2];
    function void f();
      c[1] = '{7, 8};
      c[0] = '{default: 3};
    endfunction
  endclass
  int c [2][2];
  initial begin
    C o;
    o = new;
    o.f();
    $display("cls=%0d %0d %0d %0d", o.c[0][0], o.c[0][1], o.c[1][0], o.c[1][1]);
    $display("mod=%0d %0d %0d %0d", c[0][0], c[0][1], c[1][0], c[1][1]);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["cls=3 3 7 8", "mod=0 0 0 0"]);
}
