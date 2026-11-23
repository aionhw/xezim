//! §11.3.2, §11.4.2, §13.4: an operand of a binary operator that selects an
//! element through an index with a side effect (`a[k++] == a[1]`,
//! `-a[k++] + a[1]`, `(a[k++] + 1) * a[1]`) evaluates the index once, so `k++`
//! advances `k` by one and the operator reads the element that index selects.
//! The struct, array and pattern comparisons and the width probes of the
//! operator had each evaluated the operand again, so `k` advanced twice and
//! the compare read `a[1]` against itself (#1498). Expected values were
//! checked on the reference simulator.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("T|") || l.starts_with("SIDE"))
        .collect()
}

#[test]
fn issue_repro_compares_the_selected_elements() {
    let lines = run(r#"
module top;
  int k;
  function automatic void f();
    int a[];
    bit r;
    a = '{3, 1};
    k = 0;
    r = a[k++] == a[1];
    $display("SIDE r=%0d k=%0d", r, k);
    k = 0;
    r = a[k++] == 3;
    $display("SIDE2 r=%0d k=%0d", r, k);
  endfunction
  initial f();
endmodule
"#);
    assert_eq!(lines, ["SIDE r=0 k=1", "SIDE2 r=1 k=1"]);
}

#[test]
fn every_operator_and_collection_runs_the_index_once() {
    let lines = run(r#"
module top;
  int k, c;
  int fa[2] = '{3, 1};
  int q[$] = '{3, 1};
  int aa[int];
  int da[];
  byte bq[$] = '{8'd3, 8'd1};
  logic [7:0] lv [2] = '{8'd3, 8'd1};
  function automatic int nx(); c++; return k++; endfunction
  function automatic void f();
    int a[];
    bit r; int s;
    a = '{3, 1};
    k = 0; r = a[k++] != a[1]; $display("T|ne r=%0d k=%0d", r, k);
    k = 0; r = a[k++] < a[1]; $display("T|lt r=%0d k=%0d", r, k);
    k = 0; r = a[k++] === a[1]; $display("T|ceq r=%0d k=%0d", r, k);
    k = 0; s = a[k++] + a[1]; $display("T|add s=%0d k=%0d", s, k);
    k = 0; s = a[k++] - a[1]; $display("T|sub s=%0d k=%0d", s, k);
    k = 0; r = a[k++] == a[k++]; $display("T|both r=%0d k=%0d", r, k);
    k = 0; r = a[0] == a[k++]; $display("T|rhs r=%0d k=%0d", r, k);
    k = 0; r = q[k++] == q[1]; $display("T|q r=%0d k=%0d", r, k);
    k = 0; r = fa[k++] == fa[1]; $display("T|fa r=%0d k=%0d", r, k);
    k = 0; r = bq[k++] == bq[1]; $display("T|bq r=%0d k=%0d", r, k);
    k = 0; r = lv[k++] == lv[1]; $display("T|lv r=%0d k=%0d", r, k);
    aa[0] = 3; aa[1] = 1;
    k = 0; r = aa[k++] == aa[1]; $display("T|aa r=%0d k=%0d", r, k);
    k = 0; c = 0; r = a[nx()] == a[1]; $display("T|call r=%0d k=%0d c=%0d", r, k, c);
    k = 0; r = (a[k++] == a[1]) && 1; $display("T|and r=%0d k=%0d", r, k);
    k = 0; r = a[++k] == a[0]; $display("T|pre r=%0d k=%0d", r, k);
    k = 0; s = a[k++] * a[1] + a[0]; $display("T|mul s=%0d k=%0d", s, k);
    k = 0; r = a[k++] > a[1]; $display("T|gt r=%0d k=%0d", r, k);
    k = 0; s = a[k++] & a[1]; $display("T|band s=%0d k=%0d", s, k);
    k = 0; r = a[(k+=1)] == a[1]; $display("T|asg r=%0d k=%0d", r, k);
  endfunction
  initial begin
    f();
    da = '{3, 1};
    k = 0; $display("T|mod r=%0d k=%0d", da[k++] == da[1], k);
    k = 0; c = (q[k++] == q[1]); $display("T|modq r=%0d k=%0d", c, k);
    k = 0; c = (fa[k++] == fa[1]); $display("T|modfa r=%0d k=%0d", c, k);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "T|ne r=1 k=1",
            "T|lt r=0 k=1",
            "T|ceq r=0 k=1",
            "T|add s=4 k=1",
            "T|sub s=2 k=1",
            "T|both r=0 k=2",
            "T|rhs r=1 k=1",
            "T|q r=0 k=1",
            "T|fa r=0 k=1",
            "T|bq r=0 k=1",
            "T|lv r=0 k=1",
            "T|aa r=0 k=1",
            "T|call r=0 k=1 c=1",
            "T|and r=0 k=1",
            "T|pre r=0 k=1",
            "T|mul s=6 k=1",
            "T|gt r=1 k=1",
            "T|band s=1 k=1",
            "T|asg r=1 k=1",
            "T|mod r=0 k=1",
            "T|modq r=0 k=1",
            "T|modfa r=0 k=1",
        ]
    );
}

#[test]
fn aggregate_real_string_and_nested_operands_run_the_index_once() {
    let lines = run(r#"
module top;
  typedef struct { int x; int y; } pt_t;
  int k;
  class C; int v; function new(int i); v = i; endfunction endclass
  function automatic void f();
    pt_t sa[] = '{'{1, 2}, '{3, 4}};
    string ss[] = '{"ab", "cd"};
    real ra[] = '{1.5, 2.5};
    int a[] = '{3, 1};
    int m[2][2] = '{'{1, 2}, '{3, 4}};
    C cs[2];
    bit r; int s; real rr;
    cs[0] = new(5); cs[1] = new(6);
    k = 0; r = sa[k++] == sa[1]; $display("T|struct r=%0d k=%0d", r, k);
    k = 0; r = ss[k++] == ss[1]; $display("T|str r=%0d k=%0d", r, k);
    k = 0; r = ss[k++] < ss[1]; $display("T|strlt r=%0d k=%0d", r, k);
    k = 0; rr = ra[k++] + ra[1]; $display("T|real rr=%0.1f k=%0d", rr, k);
    k = 0; r = ra[k++] == ra[1]; $display("T|realeq r=%0d k=%0d", r, k);
    k = 0; r = m[k++] == m[1]; $display("T|row r=%0d k=%0d", r, k);
    k = 0; r = m[k++][0] == m[1][0]; $display("T|m2 r=%0d k=%0d", r, k);
    k = 0; r = cs[k++] == cs[1]; $display("T|obj r=%0d k=%0d", r, k);
    k = 0; r = cs[k++].v == cs[1].v; $display("T|objv r=%0d k=%0d", r, k);
    k = 0; r = a[k++] inside {a[1]}; $display("T|inside r=%0d k=%0d", r, k);
    k = 0; s = a[k++] == a[1] ? 7 : 8; $display("T|cond s=%0d k=%0d", s, k);
    k = 0; s = -a[k++] + a[1]; $display("T|neg s=%0d k=%0d", s, k);
    k = 0; s = (a[k++] + 1) * a[1]; $display("T|nest s=%0d k=%0d", s, k);
    k = 0; s = a[k++] << a[1]; $display("T|shl s=%0d k=%0d", s, k);
    k = 0; s = a[k++] / a[1]; $display("T|div s=%0d k=%0d", s, k);
    k = 0; s = a[k++] % 2 + a[1]; $display("T|mod s=%0d k=%0d", s, k);
    k = 0; s = {a[k++], a[1]} >> 32; $display("T|cat s=%0d k=%0d", s, k);
    k = 0; r = a[k++] ==? a[1]; $display("T|wild r=%0d k=%0d", r, k);
    k = 0; r = a[k--+1] == a[1]; $display("T|dec r=%0d k=%0d", r, k);
  endfunction
  initial f();
endmodule
"#);
    assert_eq!(
        lines,
        [
            "T|struct r=0 k=1",
            "T|str r=0 k=1",
            "T|strlt r=1 k=1",
            "T|real rr=4.0 k=1",
            "T|realeq r=0 k=1",
            "T|row r=0 k=1",
            "T|m2 r=0 k=1",
            "T|obj r=0 k=1",
            "T|objv r=0 k=1",
            "T|inside r=0 k=1",
            "T|cond s=8 k=1",
            "T|neg s=-2 k=1",
            "T|nest s=4 k=1",
            "T|shl s=6 k=1",
            "T|div s=3 k=1",
            "T|mod s=2 k=1",
            "T|cat s=3 k=1",
            "T|wild r=0 k=1",
            "T|dec r=1 k=-1",
        ]
    );
}
