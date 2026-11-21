//! §19.5 bins: fixed-size array bins `name[N]` (they used to act like
//! `name[]`), transition bins with value sets, lists and repetition
//! (`[*n]`, `[->n]`, `[=n]` never hit, and `[->n]` / `[=n]` printed an
//! unknown-system-task warning), an ignored value inside an array bin (it
//! used to be recorded as a hit), and a reserved word as a bin name (it
//! used to drop the bin silently). Every expected number was cross-checked
//! against the reference simulator.

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

/// `q[4] = {[1:10], 1, 4, 7}`: 13 values, bins of 3 and the last one 4.
/// With fewer values than bins the empty bins do not count; a bin whose
/// values are all ignored is dropped.
#[test]
fn fixed_size_array_bins() {
    let l = lines(
        r#"
module top;
  bit [3:0] v;
  covergroup cg;
    cp0 : coverpoint v { bins q[4] = {[0:15]}; }
    cp1 : coverpoint v { bins q[4] = {[1:10], 1, 4, 7}; }
    cp2 : coverpoint v { bins q[4] = {1, 2}; }
    cp3 : coverpoint v { bins q[3] = {[0:5]}; ignore_bins i = {2, 3}; }
    cp4 : coverpoint v { bins q[3] = {[0:7]}; }
  endgroup
  cg c = new();
  int cov, tot;
  initial begin
    v = 1; c.sample(); v = 10; c.sample(); v = 7; c.sample();
    $display("cp0 %0.2f cp1 %0.2f cp2 %0.2f cp3 %0.2f cp4 %0.2f", c.cp0.get_inst_coverage(),
             c.cp1.get_inst_coverage(), c.cp2.get_inst_coverage(), c.cp3.get_inst_coverage(),
             c.cp4.get_inst_coverage());
    void'(c.get_inst_coverage(cov, tot)); $display("cg %0d/%0d", cov, tot);
  end
endmodule
"#,
    );
    assert_line(&l, "cp0 75.00 cp1 75.00 cp2 50.00 cp3 50.00 cp4 66.67");
    assert_line(&l, "cg 10/15");
}

#[test]
fn transition_bins() {
    let l = lines(
        r#"
module top;
  bit [2:0] s;
  covergroup cg;
    c1 : coverpoint s { bins t = (1 => 2 [* 2] => 3); }
    c2 : coverpoint s { bins r = (1 [* 2:3]); }
    c3 : coverpoint s { bins g = (2 [-> 2] => 5); }
    c4 : coverpoint s { bins a[] = (1 => 2), (3 => 4); }
    c5 : coverpoint s { bins n = (2 [= 2] => 5); }
    c6 : coverpoint s { bins m = (1, 2 => 3, 4); bins k = (6 => 7); }
    c7 : coverpoint s { bins lo = {[0:3]}; bins t = (1 => 2); }
    c8 : coverpoint s { bins l = (1 => 7), (5 => 6); }
  endgroup
  cg c = new();
  int cov, tot;
  initial begin
    s = 1; c.sample(); s = 2; c.sample(); s = 2; c.sample(); s = 3; c.sample();
    s = 1; c.sample(); s = 1; c.sample(); s = 1; c.sample(); s = 5; c.sample();
    s = 2; c.sample(); s = 0; c.sample(); s = 2; c.sample(); s = 5; c.sample();
    s = 6; c.sample();
    $display("c1 %0.2f c2 %0.2f c3 %0.2f c4 %0.2f c5 %0.2f c6 %0.2f c7 %0.2f c8 %0.2f",
             c.c1.get_inst_coverage(), c.c2.get_inst_coverage(), c.c3.get_inst_coverage(),
             c.c4.get_inst_coverage(), c.c5.get_inst_coverage(), c.c6.get_inst_coverage(),
             c.c7.get_inst_coverage(), c.c8.get_inst_coverage());
    void'(c.c4.get_inst_coverage(cov, tot)); $display("c4 %0d/%0d", cov, tot);
  end
endmodule
"#,
    );
    assert_line(
        &l,
        "c1 100.00 c2 100.00 c3 100.00 c4 50.00 c5 100.00 c6 50.00 c7 100.00 c8 100.00",
    );
    assert_line(&l, "c4 1/2");
}

/// Goto and non-consecutive repetition after a first step.
#[test]
fn goto_and_nonconsecutive_transitions() {
    let sim = simulate(
        r#"
module tb;
  bit [2:0] s;
  covergroup cg_rep;
    cp_goto : coverpoint s { bins g = (1 => 3 [-> 2]); }
    cp_nc   : coverpoint s { bins n = (1 => 3 [= 2]); }
  endgroup
  cg_rep c3 = new();
  initial begin
    s = 1; c3.sample(); s = 3; c3.sample(); s = 2; c3.sample(); s = 3; c3.sample();
    $display("goto %0.2f nonconsec %0.2f", c3.cp_goto.get_inst_coverage(), c3.cp_nc.get_inst_coverage());
  end
endmodule
"#,
        100_000,
    )
    .expect("simulate failed");
    let l: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_line(&l, "goto 100.00 nonconsec 100.00");
    assert!(
        !l.iter().any(|m| m.contains("unknown system task")),
        "{l:?}"
    );
}

/// A value ignored by `ignore_bins` is in no bin, including an array bin.
#[test]
fn ignored_value_hits_no_array_bin() {
    let sim = simulate(
        r#"
module tb;
  bit [1:0] m;
  covergroup cg;
    cp_m : coverpoint m { bins v[] = {[0:3]}; ignore_bins r = {3}; }
  endgroup
  cg c = new();
  initial begin
    m = 3; c.sample(); m = 0; c.sample();
    $display("cp_m %0.2f", c.cp_m.get_inst_coverage());
  end
endmodule
"#,
        100_000,
    )
    .expect("simulate failed");
    let l: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_line(&l, "cp_m 33.33");
    assert_eq!(sim.coverpoint_bin_hits("cg", "cp_m.v[0]"), 1);
    assert_eq!(sim.coverpoint_bin_hits("cg", "cp_m.v[3]"), 0);
}

/// §19.5: a bin is named by an identifier; `small` is a reserved word.
#[test]
fn reserved_word_bin_name_is_an_error() {
    let err = simulate(
        r#"
module tb;
  bit [1:0] size;
  covergroup cg;
    cp_size : coverpoint size { bins small = {0, 1}; bins large = {2, 3}; }
  endgroup
  cg c = new();
endmodule
"#,
        100,
    )
    .err()
    .expect("a reserved bin name must not parse");
    assert!(err.contains("expected a bin name, found 'small'"), "{err}");
}

/// §6.24.1 and §19.5: the real cast belongs to the sampled expression.
#[test]
fn sample_real_cast_coverpoint() {
    let output = lines(
        r#"
module top;
  int x;
  covergroup cg;
    cp: coverpoint real'(x) { bins three = {3.0}; }
    seen: coverpoint x { bins three = {3}; }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 3;
    #1;
    c.sample();
    $display("sampled=%0.0f", c.seen.get_inst_coverage());
    $finish;
  end
endmodule
"#,
    );
    assert_line(&output, "sampled=100");
}
