//! §6.18 / §7.4 / §13.4.1: an unpacked-array typedef declared INSIDE a child
//! module (`typedef logic [7:0] arr_t [N];`) keeps its dimensions in every
//! instance. Inlining recorded only the element width and type, so each
//! `arr_t v;` in the instance — a module variable, a block local, a function
//! return — became a scalar of the element width: writes to `v[i]` landed in
//! bits of that scalar and a returned array reached the caller as 0 or x.
//! The same module run as the top worked.

fn output(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 1_000_000).expect("simulation failed");
    assert_eq!(result.error_count, 0, "unexpected simulation error");
    let mut lines: Vec<String> = result
        .output
        .iter()
        .filter_map(|o| o.message.strip_prefix("T|").map(str::to_string))
        .collect();
    lines.sort();
    lines
}

#[test]
fn child_typedef_arrays_per_instance() {
    let got = output(
        r#"
typedef int arr_t [0:1];
module sub #(parameter int N = 3);
  typedef logic [7:0] arr_t [N];
  typedef arr_t alias_t;
  function automatic arr_t f(input int k);
    for (int i = 0; i < N; i++) f[i] = k + i;
  endfunction
  arr_t m;
  alias_t al;
  initial begin
    arr_t a;
    alias_t b;
    a = f(10);
    m = f(20);
    for (int i = 0; i < N; i++) b[i] = 30 + i;
    al = b;
    $display("T|N=%0d sz %0d %0d %0d %0d a %0d m %0d b %0d al %0d", N,
             $size(a), $size(m), $size(b), $size(al), a[N-1], m[N-1], b[N-1], al[N-1]);
  end
endmodule
module other;
  arr_t x;
  initial $display("T|other sz %0d", $size(x));
endmodule
module top;
  sub u3();
  sub #(.N(5)) u5();
  other o();
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "N=3 sz 3 3 3 3 a 12 m 22 b 32 al 32",
            "N=5 sz 5 5 5 5 a 14 m 24 b 34 al 34",
            "other sz 2",
        ],
        "{got:?}"
    );
}

#[test]
fn child_function_returns_typedef_array() {
    let got = output(
        r#"
module sub;
  typedef logic signed [31:0] arr_t [2:4];
  function automatic arr_t loop_local(input logic signed [31:0] k);
    logic signed [31:0] v [2:4];
    for (int i = 2; i <= 4; i++) v[i] = k * i;
    return v;
  endfunction
  function automatic arr_t typed_local(input logic signed [31:0] k);
    arr_t v;
    v[2] = k * 2; v[3] = k * 3; v[4] = k * 4;
    return v;
  endfunction
  initial begin
    arr_t a, b;
    a = loop_local(2);
    b = typed_local(3);
    $display("T|a %0d %0d %0d b %0d %0d %0d", a[2], a[3], a[4], b[2], b[3], b[4]);
  end
endmodule
module top;
  sub dut();
endmodule
"#,
    );
    assert_eq!(got, ["a 4 6 8 b 6 9 12"], "{got:?}");
}
