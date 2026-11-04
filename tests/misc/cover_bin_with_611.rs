//! IEEE 1800-2017 §19.5, §19.5.1.1: a coverpoint bin may end with
//! `with (expr)` and `iff (expr)`, after values, `default` or a transition
//! list, with any expression inside (`m inside {1, 2}`). `with` was a parse
//! error that took the rest of the covergroup with it (#611, reported by
//! git-hevangel); it now keeps only the values it admits, `item` bound to
//! each.
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
fn reported_declarations() {
    let out = shown(
        r#"// IEEE 1800-2017 19.5: a coverpoint bin ends with optional `with ( expression )`
// and `iff ( expression )` clauses, after a value list, after `default` and
// after a transition list; any expression may stand in the parentheses,
// including one that contains braces (`m inside {1, 2}`).
//
// Every sample below has m inside {1, 2}, so the guards are true, and the
// values 4, 5 and 6 are all sampled, so every bin is hit whether or not a
// simulator evaluates `with` and `iff`: this file checks that the declarations
// are accepted and the covergroup runs. (Evaluation of the clauses is
// repro_guard.sv.)
//
// Every line should print PASS. With the bug the file does not compile:
//   error: unexpected token in covergroup: ")"
// option.per_instance = 1 makes get_inst_coverage() report each instance (19.8).
module t;
  logic [3:0] x;
  logic [3:0] m;
  int fails = 0;

  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x {
      bins b           = {2}       iff (m inside {1, 2});
      ignore_bins c    = {0, 1}    iff (m inside {1, 2});
      bins sel[]      = {4, 5, 6} with (item inside {5, 6});
      bins t           = (7 => 8)  iff (m inside {1, 2});
      bins other       = default   iff (m inside {1, 2});
    }
  endgroup

  task automatic check(string label, real got, real expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0.1f, expected %0.1f", label, got, expected); fails++; end
  endtask

  initial begin
    cg c = new();
    m = 1;
    x = 2; c.sample();
    x = 0; c.sample();                  // ignored
    x = 4; c.sample();                  // outside `with`, in the default bin
    x = 5; c.sample();
    x = 6; c.sample();
    x = 7; c.sample();
    m = 2;
    x = 8; c.sample();
    x = 9; c.sample();                  // falls in the default bin
    check("coverage with every counted bin hit", c.get_inst_coverage(), 100.0);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
}

/// `with` drops values from an array bin (4 is not `sel[4]`) and from a
/// scalar one; a false `iff` does not count.
#[test]
fn with_and_iff_are_evaluated() {
    assert_eq!(
        shown(
            r#"module t;
  logic [3:0] x, m;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x {
      bins sel[] = {4, 5, 6} with (item inside {5, 6});
      bins ev = {[8:11]} with (item % 2 == 0);
      bins g = {2} iff (m inside {1, 2});
    }
  endgroup
  cg c = new;
  initial begin
    m = 3;
    x = 4; c.sample();  // filtered out of sel
    x = 2; c.sample();  // guard false
    $display("A %0.2f", c.cp.get_inst_coverage());
    x = 5; c.sample(); x = 9; c.sample();  // 9 odd: not in ev
    $display("B %0.2f", c.cp.get_inst_coverage());
    x = 6; c.sample(); x = 10; c.sample(); m = 1; x = 2; c.sample();
    $display("C %0.2f", c.cp.get_inst_coverage());
  end
endmodule
"#,
            |m| m.starts_with("A ") || m.starts_with("B ") || m.starts_with("C ")
        ),
        ["A 0.00", "B 25.00", "C 100.00"]
    );
}

/// §19.5.1.1: the filter selects the values before anything else uses them:
/// an `ignore_bins`/`illegal_bins` excludes only the values it admits, a
/// `name[N]` bin spreads only them, and a scalar bin they all fail is not a
/// bin.
#[test]
fn filter_applies_to_every_bin_kind() {
    let out: Vec<String> = simulate(r#"module t;
  bit [3:0] v;
  covergroup g1; cp: coverpoint v { bins b[] = {[0:15]}; ignore_bins odd = {[0:15]} with (item % 2 == 1); } endgroup
  covergroup g2; cp: coverpoint v { bins b[4] = {[0:15]} with (item < 4); } endgroup
  covergroup g3; cp: coverpoint v { bins lo = {[0:3]}; bins none = {[8:9]} with (item > 12); } endgroup
  covergroup g4; cp: coverpoint v { bins b[] = {[0:15]}; illegal_bins bad = {[0:15]} with (item > 13); } endgroup
  g1 c1 = new; g2 c2 = new; g3 c3 = new; g4 c4 = new;
  initial begin
    for (int i = 0; i < 4; i++) begin v = i; c1.sample(); c2.sample(); c3.sample(); c4.sample(); end
    $display("R %0.2f %0.2f %0.2f %0.2f", c1.get_coverage(), c2.get_coverage(), c3.get_coverage(), c4.get_coverage());
  end
endmodule
"#, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("R "))
        .collect();
    assert_eq!(out, ["R 25.00 100.00 100.00 28.57"]);
}
