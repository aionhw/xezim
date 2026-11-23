//! §19.6: a variable named in a cross that has no coverpoint of its own gets
//! an implicit coverpoint, as if `k: coverpoint k;` were declared. It is
//! the covergroup's coverpoint, so a cross's `iff` does not gate it, and one
//! `sample()` gives it one hit however many crosses name the variable. It
//! used to be sampled by each cross: a false `iff` on the cross skipped it,
//! and two crosses gave it two hits.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn assert_line(lines: &[String], want: &str) {
    assert!(
        lines.iter().any(|l| l == want),
        "expected `{want}`, got {lines:?}"
    );
}

/// The cross's guard gates the cross only; `k` is sampled on every
/// `sample()`. The expected numbers are those of two commercial simulators.
#[test]
fn cross_iff_does_not_gate_the_implicit_coverpoint() {
    let l = lines(
        r#"
module tb;
  bit [1:0] v, k;
  bit en;
  function automatic int p(real r); return $rtoi(r * 100.0 + 0.5); endfunction
  covergroup cg;
    option.per_instance = 1;
    a:  coverpoint v { bins a0 = {0}; bins a1 = {1}; }
    ax: cross a, k iff (en);
  endgroup
  cg c = new;
  initial begin
    v = 0; k = 0; en = 0; c.sample();
    $display("s1 k=%0d ax=%0d a=%0d grp=%0d", p(c.k.get_inst_coverage()),
             p(c.ax.get_inst_coverage()), p(c.a.get_inst_coverage()), p(c.get_inst_coverage()));
    v = 1; k = 1; en = 1; c.sample();
    $display("s2 k=%0d ax=%0d a=%0d grp=%0d", p(c.k.get_inst_coverage()),
             p(c.ax.get_inst_coverage()), p(c.a.get_inst_coverage()), p(c.get_inst_coverage()));
  end
endmodule
"#,
    );
    assert_line(&l, "s1 k=2500 ax=0 a=5000 grp=2500");
    assert_line(&l, "s2 k=5000 ax=1250 a=10000 grp=5417");
}

/// Two crosses name `k`: one `sample()` is one hit on `k`, so with
/// `at_least = 2` its bin is covered only after the second sample.
#[test]
fn implicit_coverpoint_of_two_crosses_is_hit_once_per_sample() {
    let l = lines(
        r#"
module tb;
  bit [1:0] v, w, k;
  function automatic int p(real r); return $rtoi(r * 100.0 + 0.5); endfunction
  covergroup cg;
    option.at_least = 2;
    a:  coverpoint v;
    b:  coverpoint w;
    ax: cross a, k;
    bx: cross b, k;
  endgroup
  cg c = new;
  initial begin
    v = 0; w = 0; k = 0; c.sample();
    $display("s1 k=%0d", p(c.k.get_inst_coverage()));
    c.sample();
    $display("s2 k=%0d", p(c.k.get_inst_coverage()));
  end
endmodule
"#,
    );
    assert_line(&l, "s1 k=0");
    assert_line(&l, "s2 k=2500");
}

/// Guard: one cross with no `iff`, and a guarded cross whose guard is true,
/// sample their implicit coverpoints as before.
#[test]
fn unguarded_and_true_guarded_implicit_coverpoints_are_unchanged() {
    let l = lines(
        r#"
module tb;
  bit [1:0] v, k, j;
  bit en = 1;
  function automatic int p(real r); return $rtoi(r * 100.0 + 0.5); endfunction
  covergroup cg;
    a:  coverpoint v { bins a0 = {0}; bins a1 = {1}; }
    ax: cross a, k;
    aj: cross a, j iff (en);
  endgroup
  cg c = new;
  initial begin
    v = 0; k = 0; j = 0; c.sample();
    $display("s1 k=%0d j=%0d ax=%0d aj=%0d", p(c.k.get_inst_coverage()),
             p(c.j.get_inst_coverage()), p(c.ax.get_inst_coverage()), p(c.aj.get_inst_coverage()));
    k = 1; j = 1; c.sample();
    $display("s2 k=%0d j=%0d", p(c.k.get_inst_coverage()), p(c.j.get_inst_coverage()));
  end
endmodule
"#,
    );
    assert_line(&l, "s1 k=2500 j=2500 ax=1250 aj=1250");
    assert_line(&l, "s2 k=5000 j=5000");
}
