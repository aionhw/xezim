//! Review follow-ups to PR #720 (contributed by AaronKel).
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 10_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("R "))
        .collect()
}

/// §18.5.11, §18.8: a whole-object `constraint_mode(0)` also turns off a
/// static block, so `randomize(null)` no longer checks it; re-enabling the
/// block by name turns it back on.
#[test]
fn whole_object_disable_reaches_static_blocks() {
    assert_eq!(
        shown(
            r#"class p_t;
  rand int x;
  static constraint s { x == 5; }
  constraint c { x < 100; }
endclass
module t;
  initial begin
    p_t p = new;
    int r1, r2, r3;
    p.x = 6;
    r1 = p.randomize(null);
    p.constraint_mode(0);
    r2 = p.randomize(null);
    p.s.constraint_mode(1);
    r3 = p.randomize(null);
    $display("R %0d %0d %0d", r1, r2, r3);
  end
endmodule
"#
        ),
        ["R 0 1 0"]
    );
}
