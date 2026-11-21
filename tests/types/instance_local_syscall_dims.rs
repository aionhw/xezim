//! §6.20.2, §20.8.1, §23.10: a statement-local declaration in an
//! instantiated module whose packed dimension calls a value system function on
//! the instance's parameters (`logic [$clog2(P)-1:0] v;`) takes this
//! instance's value. The local used to keep its bare `P`, which the
//! design-wide parameter map holds only as `u.P`, and came out one bit wide in
//! a process and 32 bits wide in a function, while `[P-1:0]` was right. A
//! keccak `theta()` declaring `logic [$clog2(W)-1:0] index_z;` inside its loop
//! computed every SHA3 digest wrong.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn instance_local_syscall_dims_take_the_instance_parameters() {
    let src = r#"
module m #(parameter int P = 16, localparam int Q = P / 4) ();
  function automatic int f_loop();
    int r;
    for (int i = 0; i < 1; i++) begin
      logic [$clog2(P)-1:0] v;
      v = '1;
      r = v;
    end
    return r;
  endfunction
  function automatic int f_top();
    logic [$clog2(Q)-1:0] v;
    v = '1;
    return v;
  endfunction
  task automatic t_width(output int w);
    logic [$signed(P)-1:0] v;
    w = $bits(v);
  endtask
  initial begin
    logic [$clog2(P)-1:0]  a;
    logic [$signed(P)-1:0] b;
    logic [$clog2(Q)-1:0]  c;
    logic [int'(P)-1:0]    d;
    int tw;
    a = '1;
    t_width(tw);
    #1 $display("P=%0d a=%0d/%0d b=%0d c=%0d d=%0d fl=%0d ft=%0d tw=%0d",
                P, $bits(a), a, $bits(b), $bits(c), $bits(d), f_loop(), f_top(), tw);
  end
endmodule
module top;
  m #(.P(256)) u256 ();
  m #(.P(64))  u64 ();
endmodule
"#;
    let out = lines(src);
    for want in [
        "P=256 a=8/255 b=256 c=6 d=256 fl=255 ft=63 tw=256",
        "P=64 a=6/63 b=64 c=4 d=64 fl=63 ft=15 tw=64",
    ] {
        assert!(
            out.iter().any(|m| m == want),
            "missing {want:?}; got {out:?}"
        );
    }
}

#[test]
fn local_declaration_query_and_parameter_value_share_a_dimension() {
    let out = lines(r#"
module leaf #(parameter int N = 16)();
  initial begin
    logic [4:0] local_value;
    logic [$bits(local_value) + $clog2(N) - 1:0] mixed;
    #1 $display("N=%0d width=%0d", N, $bits(mixed));
  end
endmodule
module top;
  leaf #(16) first();
  leaf #(64) second();
endmodule"#);
    for expected in ["N=16 width=9", "N=64 width=11"] {
        assert!(out.iter().any(|m| m == expected), "{out:?}");
    }
}
