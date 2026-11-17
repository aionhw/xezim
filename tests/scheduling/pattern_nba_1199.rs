//! IEEE 1800-2023 §10.4.2, §4.9.4, §9.4.5: pattern leaves commit in NBA.
fn output(src: &str) -> String {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn pattern_nba_1199_swap() {
    let out = output(
        r#"module sw;
  logic clk = 0;
  int a [2] = '{1, 2};
  int b [2] = '{3, 4};
  always_ff @(posedge clk) begin
    a <= '{b[0], b[1]};
    b <= '{a[0], a[1]};
  end
  initial begin
    #1 clk = 1; #1;
    $display("a=%0d %0d b=%0d %0d", a[0], a[1], b[0], b[1]);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.contains("a=3 4 b=1 2"), "{out}");
}
#[test]
fn pattern_nba_1199_delay() {
    let out = output(
        r#"module a;
  typedef struct { int x; int y; } s_t;
  int wn [2]; s_t st; int sc; int we [2];
  initial begin
    wn <= #1 '{3, 4};
    st <= #1 '{5, 6};
    sc <= #1 7;
    we[0] <= #1 8;
    #1;
    $display("t1 active: wn=%0d %0d st=%0d %0d sc=%0d we0=%0d", wn[0], wn[1], st.x, st.y, sc, we[0]);
    #0;
    $display("t1 #0: wn=%0d %0d st=%0d %0d sc=%0d we0=%0d", wn[0], wn[1], st.x, st.y, sc, we[0]);
    #1;
    $display("t2: wn=%0d %0d st=%0d %0d sc=%0d we0=%0d", wn[0], wn[1], st.x, st.y, sc, we[0]);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.contains("t1 active: wn=0 0 st=0 0 sc=0 we0=0"), "{out}");
    assert!(out.contains("t1 #0: wn=0 0 st=0 0 sc=0 we0=0"), "{out}");
    assert!(out.contains("t2: wn=3 4 st=5 6 sc=7 we0=8"), "{out}");
}

#[test]
fn pattern_nba_1199_nested_and_side_effects() {
    let out = output(
        r#"
module top;
  typedef struct { int x; int y; } pair_t;
  pair_t a, b, cells[2];
  int matrix[2][2];
  int side[2];
  int calls;
  function int next_value();
    side = '{7, 8};
    calls++;
    return calls;
  endfunction
  initial begin
    a = '{1, 2}; b = '{3, 4}; cells = '{default:'{default:0}};
    matrix = '{default:'{default:0}};
    a <= '{b.x, b.y}; b <= '{a.x, a.y};
    cells[1] <= '{x:next_value(), y:next_value()};
    matrix <= '{'{1:9, default:2}, '{2{5}}};
    if (calls != 2 || side[0] != 7 || side[1] != 8) $fatal(1, "FAIL side effects");
    if (a.x != 1 || b.x != 3 || cells[1].x != 0 || matrix[0][1] != 0) $fatal(1, "FAIL active");
    #0;
    if (a.x != 1 || matrix[0][1] != 0) $fatal(1, "FAIL inactive");
    #1;
    if (a.x != 3 || a.y != 4 || b.x != 1 || b.y != 2) $fatal(1, "FAIL swap");
    if (cells[1].x != 1 || cells[1].y != 2 || cells[0].x != 0) $fatal(1, "FAIL cells");
    if (matrix[0][0] != 2 || matrix[0][1] != 9 || matrix[1][0] != 5 || matrix[1][1] != 5) $fatal(1, "FAIL matrix");
    $display("PASS nested");
  end
endmodule
"#,
    );
    assert!(
        !out.contains("FAIL") && out.contains("PASS nested"),
        "{out}"
    );
}
