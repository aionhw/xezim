//! IEEE 1800-2017 §10.9.1 and §6.21, as iverilog rejects them: an
//! ordered assignment pattern must have one item per element of the
//! unpacked dimension it fills (`'{1,2,3}` for `[1:4]`), and `$monitor` may
//! not take an automatic variable, which no longer exists when the monitor
//! prints (a `for` loop variable, a variable declared `automatic`, a local
//! of an automatic task). xezim accepted both, and the monitor printed x.
use xezim::simulate;

fn error_of(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn reported_two_dimensional_array_testbench() {
    let e = error_of(
        r#"module array_md;
 int upk_2d [1:2] [1:4] ;
 initial begin
 upk_2d='{'{1,2,3}, '{4,5,6}};
 for(int i=1;i<3;i++)
 for(int j=1;j<4;j++)
 begin
 upk_2d[i][j]=$random;
 $monitor("%s array upk_2d[%0d][%0d]:%0d",,i,j,upk_2d[i][j]);
 end
 end
endmodule
"#,
    );
    for want in [
        "automatic variable 'i' is not allowed as an argument to $monitor",
        "automatic variable 'j' is not allowed as an argument to $monitor",
        "assignment pattern for 'upk_2d' has 3 element(s), but the dimension [1:4] has 4",
    ] {
        assert!(e.contains(want), "{e}");
    }
}

#[test]
fn pattern_counts_whole_and_by_row() {
    let e = error_of(
        r#"module tb;
  int v [3];
  int w [2][3];
  initial begin
    v = '{1, 2};
    w[1] = '{1, 2, 3, 4};
  end
endmodule
"#,
    );
    assert!(
        e.contains("pattern for 'v' has 2 element(s), but the dimension [0:2] has 3"),
        "{e}"
    );
    assert!(
        e.contains("pattern for 'w' has 4 element(s), but the dimension [0:2] has 3"),
        "{e}"
    );
}

#[test]
fn monitor_of_automatic_locals() {
    let e = error_of(
        r#"module tb;
  int arr [2];
  task automatic t(input int p); int k = 3; $monitor("%0d %0d", k, p); endtask
  initial begin
    automatic int y = 5;
    $monitor("%0d", y);
    foreach (arr[b]) $monitor("%0d", b);
  end
endmodule
"#,
    );
    for v in ["'k'", "'p'", "'y'", "'b'"] {
        assert!(e.contains(&format!("automatic variable {v}")), "{e}");
    }
}

/// Legal forms stay legal: static and module variables in `$monitor`, an
/// automatic variable in `$display`/`$strobe`, matching, `default:` and
/// replicated patterns, and a name redeclared in a block.
#[test]
fn legal_forms_are_accepted() {
    let r = simulate(
        r#"module tb;
  int x;
  int u [1:2][1:4];
  int q [3];
  task t(); static int s = 1; $monitor("%0d", s); endtask
  initial begin
    static int st = 2;
    automatic int au = 3;
    int u [2];
    u = '{7, 8};
    q = '{default: 1};
    q = '{3{4}};
    $monitor("%0d %0d", x, st);
    $display("%0d", au);
    $strobe("%0d", au);
    for (int i = 0; i < 1; i++) $display("%0d", i);
  end
  initial tb.u = '{'{1,2,3,4}, '{5,6,7,8}};
endmodule
"#,
        10,
    );
    assert!(r.is_ok(), "{:?}", r.err().map(|e| e.to_string()));
}
