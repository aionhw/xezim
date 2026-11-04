//! IEEE 1800-2017 §7.5.1, §7.6, §13.5.1: a dynamic array of dynamic arrays
//! (or of queues) copied by `new[n](src)`, by whole assignment, and into a
//! by-value argument keeps each row's size and elements. `new[n](src)` on a
//! class member cleared every row and on a module array read each row as x
//! (#595, reported by akashlevy); a by-value `int o[][]` argument arrived
//! with every row empty, and a whole assignment into a class member copied
//! no rows.
use xezim::simulate;

fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

#[test]
fn reported_testbench() {
    let src = r#"// §7.5.1: new[n](src) keeps min(n, src.size()) elements of src; a row of a
// dynamic array of dynamic arrays is copied with its size and elements.
class C;
  int d[][];
  int o[][];
  function void run();
    o = new[2];
    o[0] = new[2];
    o[0][0] = 5;
    o[0][1] = 6;
    o[1] = new[1];
    o[1][0] = 7;
    d = new[3](o);
    $display("cls d=%0d %0d %0d %0d %0d %0d expect 3 2 5 6 7 0",
             d.size(), d[0].size(), d[0][0], d[0][1], d[1][0], d[2].size());
  endfunction
endclass
module t;
  int md[][];
  int mo[][];
  initial begin
    C c = new;
    c.run();
    mo = new[2];
    mo[0] = new[2];
    mo[0][0] = 5;
    mo[0][1] = 6;
    mo[1] = new[1];
    mo[1][0] = 7;
    md = new[3](mo);
    $display("mod d=%0d %0d %0d %0d %0d %0d expect 3 2 5 6 7 0",
             md.size(), md[0].size(), md[0][0], md[0][1], md[1][0], md[2].size());
  end
endmodule

"#;
    assert_eq!(
        shown(src, &["cls ", "mod "]),
        [
            "cls d=3 2 5 6 7 0 expect 3 2 5 6 7 0",
            "mod d=3 2 5 6 7 0 expect 3 2 5 6 7 0"
        ]
    );
}

/// Truncation, queue rows, a self-resize, an argument source, and a copy
/// independent of its source.
#[test]
fn constructor_copies_rows() {
    let src = r#"class C;
  int d[][];
  int q[][$];
  function void run(int o[][]);
    d = new[1](o); $display("A %0d %0d %0d", d.size(), d[0].size(), d[0][1]);
    q = new[2]; q[0].push_back(4); q[0].push_back(5); q[1].push_back(6);
    q = new[3](q); $display("B %0d %0d %0d %0d %0d", q.size(), q[0].size(), q[0][1], q[1][0], q[2].size());
  endfunction
endclass
module t;
  int md[][], mo[][], m3[][][], n3[][][];
  int mq[][$], nq[][$];
  initial begin
    C c = new;
    mo = new[2]; mo[0] = new[2]; mo[0][0] = 5; mo[0][1] = 6; mo[1] = new[1]; mo[1][0] = 7;
    c.run(mo);
    md = new[1](mo); md[0][0] = 99; $display("C %0d %0d %0d %0d", md.size(), md[0].size(), mo[0][0], md[0][0]);
    mo = new[3](mo); $display("D %0d %0d %0d %0d", mo.size(), mo[0][1], mo[1][0], mo[2].size());
    mq = new[2]; mq[0].push_back(1); mq[1].push_back(2); mq[1].push_back(3);
    nq = new[2](mq); nq[1].push_back(4); $display("E %0d %0d %0d %0d", nq[1].size(), mq[1].size(), nq[1][2], nq[0][0]);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["A ", "B ", "C ", "D ", "E "]),
        [
            "A 1 2 6",
            "B 3 2 5 6 0",
            "C 1 2 5 99",
            "D 3 6 7 0",
            "E 3 2 4 1"
        ]
    );
}

/// A by-value argument and a `ref` one.
#[test]
fn by_value_argument_gets_its_rows() {
    let src = r#"module t;
  int mo[][];
  int mq[][$];
  function automatic void f(int o[][], int q[][$]);
    $display("F %0d %0d %0d | %0d %0d %0d", o.size(), o[0].size(), o[0][1], q.size(), q[1].size(), q[1][0]);
  endfunction
  task automatic g(ref int o[][]);
    $display("G %0d %0d %0d", o.size(), o[0].size(), o[0][1]);
    o[1][0] = 42;
  endtask
  initial begin
    mo = new[2]; mo[0] = new[2]; mo[0][0] = 5; mo[0][1] = 6; mo[1] = new[1]; mo[1][0] = 7;
    mq = new[2]; mq[1].push_back(9);
    f(mo, mq);
    g(mo); $display("H %0d", mo[1][0]);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["F ", "G ", "H "]),
        ["F 2 2 6 | 2 1 9", "G 2 2 6", "H 42"]
    );
}

/// Whole assignment into a class member and between module arrays.
#[test]
fn whole_assignment_copies_rows() {
    let src = r#"class C;
  int d[][];
  int q[$];
  function void run(int o[][]);
    d = o; $display("W %0d %0d %0d", d[0].size(), d[0][1], d[1][0]);
    d[0][1] = 77; $display("X %0d %0d", d[0][1], o[0][1]);
  endfunction
endclass
module t;
  int mo[][], md[][];
  int q1[$], q2[$];
  initial begin
    C c = new;
    mo = new[2]; mo[0] = new[2]; mo[0][0] = 5; mo[0][1] = 6; mo[1] = new[1]; mo[1][0] = 7;
    c.run(mo);
    md = mo; md[1][0] = 70; $display("Y %0d %0d %0d %0d", md.size(), md[0][1], md[1][0], mo[1][0]);
    q1.push_back(3); q1.push_back(4); q2 = q1; q2[0] = 9; $display("Z %p %p", q1, q2);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["W ", "X ", "Y ", "Z "]),
        ["W 2 6 7", "X 77 6", "Y 2 6 70 7", "Z '{3, 4} '{9, 4}"]
    );
}
