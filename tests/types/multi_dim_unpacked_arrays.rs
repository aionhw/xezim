//! Multi-dimensional unpacked arrays as values: declaration initializers at
//! module scope, inside an inlined instance and inside a generate block
//! (including `'{default:}` and a 3-D pattern); whole-array copy, row read /
//! write, equality and `%p`; and 2-D / 3-D task formals (`input`, `output`,
//! `ref`) with `foreach`, `$size`, whole-pattern assignment, chained `ref`
//! passing and element writes. Every one of these read zero / x before:
//! the initializer was dropped for any shape past one dimension (and for
//! every array inside an instance), pattern assignment to a dotted array
//! reclassified it as dynamic, the copy arms knew one dimension, and a 2-D
//! formal was bound with no storage at all.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
module sub;
  int v[2][2] = '{'{1,2},'{3,4}};
  int s[2] = '{9,8};
endmodule
module top;
  int a[2][2] = '{'{1,2},'{3,4}};
  logic [7:0] l[2][2] = '{'{1,2},'{3,4}};
  int d[2][2] = '{default:7};
  int t3[2][2][2] = '{'{'{1,2},'{3,4}},'{'{5,6},'{7,8}}};
  int w[2][2];
  int r[2];
  sub u();
  if (1) begin : g
    int gw[2][2] = '{'{5,6},'{7,8}};
  end
  task automatic t_ref(ref int x[2][2]);
    int sum = 0;
    foreach (x[i,j]) sum += x[i][j];
    $display("REF sum=%0d size=%0d,%0d p=%p", sum, $size(x), $size(x,2), x);
    x[0][1] = 42;
  endtask
  task automatic t_in(input int x[2][2]); $display("IN %0d", x[1][0]); x[1][0] = 0; endtask
  task automatic t_out(output int x[2][2]); x = '{'{10,11},'{12,13}}; endtask
  task automatic t_inner(ref int x[2][2]); x[1][1] = 77; endtask
  task automatic t_outer(ref int x[2][2]); t_inner(x); $display("CHAIN in-outer %0d", x[1][1]); endtask
  task automatic t_3d(ref int x[2][2][2]); x[1][0][1] = 66; endtask
  initial begin
    #1;
    $display("INIT a=%0d %0d %0d %0d l=%0d %0d d=%0d %0d t3=%0d %0d", a[0][0], a[0][1], a[1][0], a[1][1], l[0][1], l[1][0], d[0][1], d[1][0], t3[0][1][0], t3[1][1][1]);
    $display("SCOPED sub=%0d %0d %0d gen=%0d %0d", u.v[0][1], u.v[1][0], u.s[1], g.gw[0][1], g.gw[1][0]);
    w = a; $display("COPY w=%0d %0d %0d %0d eq=%0d", w[0][0], w[0][1], w[1][0], w[1][1], w == a);
    r = a[1]; $display("ROW r=%0d %0d", r[0], r[1]);
    r[0] = 20; r[1] = 21; w[0] = r; $display("ROWW w0=%0d %0d neq=%0d", w[0][0], w[0][1], w != a);
    t_ref(a); $display("REFW a01=%0d", a[0][1]);
    t_in(a); $display("INK a10=%0d", a[1][0]);
    t_out(w); $display("OUT w=%0d %0d", w[0][0], w[1][1]);
    t_outer(a); $display("CHAIN a11=%0d", a[1][1]);
    t_3d(t3); $display("T3 %0d", t3[1][0][1]);
    $finish;
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("multi_dim_unpacked_arrays");
    std::fs::create_dir_all(&dir).unwrap();
    // One file per variant: the two variants run on parallel test threads.
    let sv = dir.join(if jit { "t_jit.sv" } else { "t_default.sv" });
    std::fs::write(&sv, DESIGN).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--simulate", "-s", "top", "--no-cache", sv.to_str().unwrap()]);
    if jit {
        cmd.env("XEZIM_JIT", "1");
    }
    let output = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn check(text: &str) {
    for want in [
        "INIT a=1 2 3 4 l=2 3 d=7 7 t3=3 8",
        "SCOPED sub=2 3 8 gen=6 7",
        "COPY w=1 2 3 4 eq=1",
        "ROW r=3 4",
        "ROWW w0=20 21 neq=1",
        "REF sum=10 size=2,2 p='{'{1, 2}, '{3, 4}}",
        "REFW a01=42",
        "IN 3",
        "INK a10=3",
        "OUT w=10 13",
        "CHAIN in-outer 77",
        "CHAIN a11=77",
        "T3 66",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}

#[test]
fn multi_dimensional_unpacked_arrays_as_values_and_formals() {
    check(&run(false));
}

#[test]
fn multi_dimensional_unpacked_arrays_as_values_and_formals_jit() {
    check(&run(true));
}
