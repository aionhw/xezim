//! §12.7.1 / §6.21: a `for` loop that declares its own loop variable and
//! writes (or reads) a fixed-size array declared in the enclosing procedural
//! block. The loop compiles to bytecode, whose array stores resolve elements
//! through the signal table; a block-local array lives in the interpreter's
//! by-name store instead, so every compiled `v[i] = ...` was dropped and the
//! array kept its initial value (0 for `int`, x for `logic`).

fn output(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 1_000_000).expect("simulation failed");
    assert_eq!(result.error_count, 0, "unexpected simulation error");
    result
        .output
        .iter()
        .filter_map(|o| o.message.strip_prefix("T|").map(str::to_string))
        .collect()
}

#[test]
fn instance_initial_local_arrays() {
    let got = output(
        r#"
module sub;
  initial begin
    int v [0:2];
    logic [31:0] w [0:2];
    automatic int a [0:2];
    int src [0:3];
    int dst [0:3];
    int sum;
    for (int i = 0; i < 3; i++) v[i] = 10 + i;
    for (int i = 0; i < 3; i++) w[i] = 20 + i;
    for (int i = 0; i < 3; i++) a[i] = 30 + i;
    for (int i = 0; i < 4; i++) src[i] = 5 * i;
    for (int i = 0; i < 4; i++) dst[3 - i] = src[i];
    for (int i = 0; i < 4; i++) sum += dst[i];
    $display("T|v %0d %0d %0d", v[0], v[1], v[2]);
    $display("T|w %0d %0d %0d", w[0], w[1], w[2]);
    $display("T|a %0d %0d %0d", a[0], a[1], a[2]);
    $display("T|dst %0d %0d sum %0d", dst[0], dst[3], sum);
  end
endmodule
module top;
  sub dut();
endmodule
"#,
    );
    assert_eq!(
        got,
        ["v 10 11 12", "w 20 21 22", "a 30 31 32", "dst 15 0 sum 30"],
        "{got:?}"
    );
}

#[test]
fn top_initial_local_array() {
    let got = output(
        r#"
module top;
  initial begin
    int v [0:2];
    for (int i = 0; i < 3; i++) v[i] = 30 + i;
    $display("T|%0d %0d %0d", v[0], v[1], v[2]);
  end
endmodule
"#,
    );
    assert_eq!(got, ["30 31 32"], "{got:?}");
}

#[test]
fn instance_subroutine_local_arrays() {
    let got = output(
        r#"
module sub #(parameter int N = 3);
  function automatic logic signed [31:0] f(input int k);
    logic signed [31:0] v [2:4];
    logic signed [31:0] s;
    for (int i = 2; i <= 4; i++) v[i] = k * i;
    s = 0;
    for (int i = 2; i <= 4; i++) s += v[i];
    return s;
  endfunction
  function automatic int g();
    int v [0:N-1];
    for (int i = 0; i < N; i++) v[i] = i * i;
    return v[N-1];
  endfunction
  task automatic t(output int o);
    int v [4];
    for (int i = 0; i < 4; i++) v[i] = 7 + i;
    o = v[3];
  endtask
  initial begin
    int r;
    t(r);
    $display("T|N=%0d f %0d g %0d t %0d", N, f(2), g(), r);
  end
endmodule
module top;
  sub dut();
  sub #(.N(5)) dut2();
endmodule
"#,
    );
    assert_eq!(got, ["N=3 f 18 g 4 t 10", "N=5 f 18 g 16 t 10"], "{got:?}");
}
