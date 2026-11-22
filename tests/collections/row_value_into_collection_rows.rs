//! §7.6, §10.9.1, §10.10: a whole row assigned to one row of an unpacked
//! array whose rows are fixed-size arrays: a row of a fixed two- or
//! three-dimensional array (`m[1] = {3, 4}`), or an element of a queue,
//! dynamic array or associative array of fixed-size arrays (`qp[0] = '{1, 2}`,
//! `aa["c"] = p`). The right-hand side is an assignment pattern, an unpacked
//! array concatenation or an array of the row type, and each item is one
//! element of the row. Whole-collection patterns whose items are rows, row
//! reads from an associative array (`r = aa["b"]`) and `%p` of an associative
//! array of rows are covered too. Expected values were checked on
//! Verilator 5.046.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn issue_repro_queue_dynamic_and_associative_rows() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  pair_t qp[$];
  pair_t da[];
  pair_t aa[string];
  pair_t p;
  initial begin
    qp = '{'{0, 0}, '{0, 0}, '{0, 0}};
    da = new[3];
    p = '{5, 6};
    qp[0] = '{1, 2};
    qp[1] = {3, 4};
    qp[2] = p;
    da[0] = '{1, 2};
    da[1] = {3, 4};
    da[2] = p;
    aa["a"] = '{1, 2};
    aa["b"] = {3, 4};
    aa["c"] = p;
    $display("R qp=%p", qp);
    $display("R da=%p", da);
    $display("R aa=%p", aa);
    $display("R qp[0][1]=%0d da[0][1]=%0d aa[\"a\"][1]=%0d", qp[0][1], da[0][1], aa["a"][1]);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R qp='{'{1, 2}, '{3, 4}, '{5, 6}}",
            "R da='{'{1, 2}, '{3, 4}, '{5, 6}}",
            "R aa='{\"a\":'{1, 2}, \"b\":'{3, 4}, \"c\":'{5, 6}}",
            "R qp[0][1]=2 da[0][1]=2 aa[\"a\"][1]=2",
        ]
    );
}

#[test]
fn concatenation_to_a_row_of_a_fixed_array() {
    let lines = run(r#"
module tb;
  int m[3][2];
  int p[2];
  int mn[3][2];
  byte mb[2][2];
  int md[2][2:1];
  int m3[2][2][2];
  int i;
  initial begin
    p = '{5, 6};
    m[0] = '{1, 2};
    m[1] = {3, 4};
    m[2] = p;
    $display("R m=%p", m);
    i = 2;
    m[i] = {i, i + 5};
    m[i - 2] = {p[1], 9};
    $display("R m=%p", m);
    mb[0] = {300, -1};
    mb[1] = {8'd7, 8'sd3};
    $display("R mb %0d %0d %0d %0d", mb[0][0], mb[0][1], mb[1][0], mb[1][1]);
    md[0] = {1, 2};
    md[1] = '{3, 4};
    $display("R md %0d %0d %0d %0d", md[0][2], md[0][1], md[1][2], md[1][1]);
    m3[1] = {'{1, 2}, '{3, 4}};
    $display("R m3=%p", m3);
    mn[0] <= '{1, 2};
    mn[1] <= {3, 4};
    mn[2] <= {p[0], i};
    #1 $display("R mn=%p", mn);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R m='{'{1, 2}, '{3, 4}, '{5, 6}}",
            "R m='{'{6, 9}, '{3, 4}, '{2, 7}}",
            "R mb 44 -1 7 3",
            "R md 1 2 3 4",
            "R m3='{'{'{0, 0}, '{0, 0}}, '{'{1, 2}, '{3, 4}}}",
            "R mn='{'{1, 2}, '{3, 4}, '{5, 2}}",
        ]
    );
}

#[test]
fn rows_of_queues_and_dynamic_arrays() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  pair_t qp[$];
  pair_t da[];
  int qi[$][2];
  int di[][2];
  pair_t p;
  int i;
  initial begin
    qp = '{'{0, 0}, '{0, 0}, '{0, 0}};
    da = new[3];
    p = '{5, 6};
    qp[0] = '{1, 2};
    qp[1] = {3, 4};
    qp[2] = p;
    da[0] = '{1, 2};
    da[1] = {3, 4};
    da[2] = p;
    $display("R qp=%p", qp);
    $display("R da=%p", da);
    $display("R qp[0][1]=%0d da[1][0]=%0d", qp[0][1], da[1][0]);
    i = 1;
    qp[i] = {i, i + 1};
    qp[$] = '{7, 8};
    qp[i - 1] = {p[1], 9};
    da[i + 1] = {p[0], 3};
    da[0] = '{default: 4};
    $display("R qp=%p", qp);
    $display("R da=%p", da);
    qp[3] = '{9, 9};
    da[3] = '{9, 9};
    $display("R qp.size=%0d da.size=%0d", qp.size(), da.size());
    qi = '{'{3, 4}, '{5, 6}};
    di = '{'{1, 2}, p};
    $display("R qi=%p di=%p sizes %0d %0d", qi, di, qi.size(), di.size());
    qi[0] = {8, 9};
    di[1] = '{11, 12};
    $display("R qi=%p di=%p", qi, di);
    foreach (qp[k]) $display("R qp[%0d] %0d %0d", k, qp[k][0], qp[k][1]);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R qp='{'{1, 2}, '{3, 4}, '{5, 6}}",
            "R da='{'{1, 2}, '{3, 4}, '{5, 6}}",
            "R qp[0][1]=2 da[1][0]=3",
            "R qp='{'{6, 9}, '{1, 2}, '{7, 8}}",
            "R da='{'{4, 4}, '{3, 4}, '{5, 3}}",
            "R qp.size=4 da.size=3",
            "R qi='{'{3, 4}, '{5, 6}} di='{'{1, 2}, '{5, 6}} sizes 2 2",
            "R qi='{'{8, 9}, '{5, 6}} di='{'{1, 2}, '{11, 12}}",
            "R qp[0] 6 9",
            "R qp[1] 1 2",
            "R qp[2] 7 8",
            "R qp[3] 9 9",
        ]
    );
}

#[test]
fn rows_of_associative_arrays() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  pair_t aa[string];
  int ai[int][2];
  pair_t p, r;
  initial begin
    p = '{5, 6};
    aa["a"] = '{1, 2};
    aa["b"] = {3, 4};
    aa["c"] = p;
    $display("R aa=%p", aa);
    $display("R n=%0d a1=%0d b0=%0d c1=%0d", aa.num(), aa["a"][1], aa["b"][0], aa["c"][1]);
    r = aa["b"];
    $display("R r=%p", r);
    aa["a"] = {p[1], 7};
    aa["b"][0] = 9;
    foreach (aa[k]) $display("R %s %0d %0d", k, aa[k][0], aa[k][1]);
    ai[3][1] = 9;
    ai[5] = '{1, 2};
    ai[8] = {7, 8};
    $display("R ai=%p n=%0d", ai, ai.num());
    if (ai.exists(5) && !ai.exists(4)) $display("R exists ok");
    ai.delete(3);
    $display("R ai=%p", ai);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R aa='{\"a\":'{1, 2}, \"b\":'{3, 4}, \"c\":'{5, 6}}",
            "R n=3 a1=2 b0=3 c1=6",
            "R r='{3, 4}",
            "R a 6 7",
            "R b 9 4",
            "R c 5 6",
            "R ai='{3:'{0, 9}, 5:'{1, 2}, 8:'{7, 8}} n=3",
            "R exists ok",
            "R ai='{5:'{1, 2}, 8:'{7, 8}}",
        ]
    );
}

#[test]
fn row_variables_as_pattern_items_and_task_local_rows() {
    let lines = run(r#"
module tb;
  typedef int pair_t[2];
  pair_t qp[$];
  pair_t p;
  task automatic t();
    pair_t lq[$];
    int lm[2][2];
    lq = '{'{0, 0}, '{0, 0}};
    lq[0] = '{21, 22};
    lq[1] = {23, 24};
    lm[1] = {25, 26};
    $display("R lq=%p lm=%p", lq, lm);
  endtask
  initial begin
    p = '{5, 6};
    qp = '{p, '{1, 2}};
    $display("R qp=%p", qp);
    t();
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R qp='{'{5, 6}, '{1, 2}}",
            "R lq='{'{21, 22}, '{23, 24}} lm='{'{0, 0}, '{25, 26}}",
        ]
    );
}

/// §7.4.6, §10.4.1: a row write with an index past the end of a dynamic
/// array or queue stores nothing, but its right-hand side still runs. §7.10.5:
/// a full bounded queue takes no append. §7.8.6: a row an associative array
/// does not hold reads as the element type's default. Values checked on
/// Verilator 5.046, except the `logic` row: Verilator 5.046 reads its x
/// default as 0.
#[test]
fn ignored_row_writes_bounded_queues_and_missing_rows() {
    let lines = run(r#"
module tb;
  int da[][2];
  int q[$][2];
  int bq[$:1][2];
  logic [7:0] la[string][2];
  int ia[string][2];
  logic [7:0] lr[2];
  int ir[2];
  int calls;
  function automatic int bump();
    calls++;
    return calls;
  endfunction
  initial begin
    da = new[1];
    q.push_back('{1, 2});
    calls = 0;
    da[5] = {bump(), 0};
    q[7] = {bump(), 5};
    da[3] = '{bump(), 5};
    $display("R calls=%0d da=%p q=%p", calls, da, q);
    bq[0] = '{1, 2};
    bq[1] = {3, 4};
    bq[2] = '{5, 6};
    bq[bq.size()] = {7, 8};
    $display("R bq=%p size=%0d", bq, bq.size());
    la["a"] = '{8'h12, 8'h34};
    ia["a"] = '{1, 2};
    ir = '{7, 7};
    ir = ia["missing"];
    $display("R ir=%p", ir);
    lr = la["missing"];
    $display("R lr=%p", lr);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R calls=3 da='{'{0, 0}} q='{'{1, 2}}",
            "R bq='{'{1, 2}, '{3, 4}} size=2",
            "R ir='{0, 0}",
            "R lr='{'hXX, 'hXX}",
        ]
    );
}

/// §10.10, §11.4.2: every operand of a concatenation written to a row is read
/// before any element of the row is written. A swap of the sub-rows of a
/// three-dimensional row reads the values from before the store (Verilator
/// 5.046 writes the second sub-row's values twice here), and a fixed row
/// among the operands contributes its elements.
#[test]
fn concatenation_operands_are_read_before_the_row_is_written() {
    let lines = run(r#"
module tb;
  int src[2][2];
  int dst[1][3];
  int m[1][2][2];
  initial begin
    src = '{'{1, 2}, '{3, 4}};
    dst = '{'{0, 0, 0}};
    dst[0] = {src[1], 9};
    $display("R dst=%p", dst);
    m[0] = '{'{1, 2}, '{3, 4}};
    m[0] = {'{m[0][1][0], m[0][1][1]}, '{m[0][0][0], m[0][0][1]}};
    $display("R m=%p", m);
  end
endmodule
"#);
    assert_eq!(lines, ["R dst='{'{3, 4, 9}}", "R m='{'{'{3, 4}, '{1, 2}}}"]);
}
