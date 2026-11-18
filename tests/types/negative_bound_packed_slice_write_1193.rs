//! IEEE 1800-2023 §7.4.1, §11.5.1: a part-select of a multidimensional
//! packed array selects ELEMENTS of its first dimension, and that dimension
//! may have negative bounds (`logic [-4:3][7:0]`, `px_t [-8:8]`). A write
//! through such a select stores the selected elements whatever the sign of
//! the labels and the direction of the dimension, and a partly out-of-range
//! select writes only its in-range elements (#1193). Covered: constant,
//! indexed and variable-base selects, ascending and descending dimensions,
//! three dimensions, nonblocking and continuous writes, `always_comb`, class
//! properties and their array elements, automatic locals, packed-struct
//! members, associative-array elements and output ports. Expected lines were
//! recorded from the reference simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with("[xezim]") && !m.starts_with("Simulation"))
        .collect()
}

/// Blocking writes in a module: constant, indexed and variable-base selects,
/// partly out-of-range selects, a descending dimension, three dimensions and
/// a typedef-built `px_t [-8:8]`.
#[test]
fn procedural_selects() {
    let src = r#"
module t1193a;
  typedef logic [9:0] px_t;
  typedef px_t [-8:8] wide_t;
  logic [-4:3][7:0] pv;
  logic [3:-4][7:0] qv;
  logic [-2:1][-1:0][3:0] x3;
  wide_t w;
  int k;
  initial begin
    pv = '0; pv[-2:0] = 24'h112233;         $display("A pv=%h", pv);
    pv = '0; pv[-4 +: 2] = 16'hBEEF;        $display("B pv=%h", pv);
    pv = '0; pv[1 -: 3] = 24'h5A5A5A;       $display("C pv=%h", pv);
    pv = '0; pv[-5:-3] = 24'hFFEEDD;        $display("D pv=%h", pv);
    pv = '0; pv[2:4] = 24'h010203;          $display("E pv=%h", pv);
    pv = '0; k = -3; pv[k +: 2] = 16'h1234; $display("F pv=%h", pv);
    qv = '0; qv[-1:-3] = 24'hCAFE01;        $display("G qv=%h", qv);
    x3 = '0; x3[-1:0] = 16'hA5C3;           $display("I x3=%h", x3);
    w = '0; w[-2:2] = {10'd1, 10'd2, 10'd3, 10'd4, 10'd5};
    $display("J w[-2]=%0d w[0]=%0d w[2]=%0d w[3]=%0d", w[-2], w[0], w[2], w[3]);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "A pv=0000112233000000",
            "B pv=beef000000000000",
            "C pv=0000005a5a5a0000",
            "D pv=eedd000000000000",
            "E pv=0000000000000102",
            "F pv=0012340000000000",
            "G qv=00000000cafe0100",
            "I x3=00a5c300",
            "J w[-2]=1 w[0]=3 w[2]=5 w[3]=0",
        ]
    );
}

/// Nonblocking (including a positive-bound multidimensional destination,
/// whose element select was written as bits), continuous and `always_comb`
/// writes.
#[test]
fn scheduled_and_continuous_writes() {
    let src = r#"
module t1193b;
  logic clk = 0;
  logic [-4:3][7:0] a, b, d;
  logic [3:0][7:0] q;
  logic [0:3][7:0] p;
  logic [15:0] src = 16'hBEEF;
  int k = -3;
  wire [-4:3][7:0] w;
  assign w[-2:0] = 24'h123456;
  always @(posedge clk) begin
    a[-3:-2] <= src;
    q[2:1] <= src;
    p[1:2] <= src;
  end
  always @(posedge clk) b[k +: 2] <= src;
  always_comb begin
    d = '0;
    d[-4:-2] = 24'h654321;
  end
  initial begin
    a = '0; b = '0; q = '0; p = '0;
    #1 clk = 1;
    #1 $display("a=%h b=%h q=%h p=%h", a, b, q, p);
    $display("d=%h w=%h", d, w);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "a=00beef0000000000 b=00beef0000000000 q=00beef00 p=00beef00",
            "d=6543210000000000 w=zzzz123456zzzzzz",
        ]
    );
}

/// Class properties written from a method and from module scope (partly
/// out of range at either end included) and their array elements, an
/// automatic local, a packed-struct member, an associative-array element and
/// an output port.
#[test]
fn class_function_struct_and_port_targets() {
    let src = r#"
module t1193c;
  typedef logic [-4:3][7:0] w_t;
  typedef struct packed { logic [3:0] tag; w_t f; } ps_t;
  class C;
    w_t p;
    w_t arr [0:1];
    function void run(int b);
      p = '0; p[b +: 3] = 24'hABCDEF;
      arr[1] = '0; arr[1][-2:0] = 24'h112233;
    endfunction
  endclass
  function automatic w_t fw(int b);
    w_t t;
    t = '0;
    t[b -: 2] = 16'hBEEF;
    return t;
  endfunction
  C c;
  ps_t s;
  w_t aa [int];
  w_t o;
  sub u(.o(o));
  initial begin
    c = new; c.run(-3);
    $display("prop=%h elem=%h", c.p, c.arr[1]);
    c.p[-4:-4] = 8'h77;
    $display("prop2=%h", c.p);
    c.p = '0; c.p[2:5] = 32'h11223344; c.p[-6:-3] = 32'hAABBCCDD;
    $display("prop3=%h", c.p);
    $display("fw=%h", fw(-3));
    s = '0; s.f[-1:1] = 24'hDEAD01;
    $display("s=%h", s);
    aa[7] = '0; aa[7][-2:-1] = 16'h4242;
    $display("aa=%h", aa[7]);
    #2 $display("o=%h", o);
  end
endmodule
module sub(output logic [-4:3][7:0] o);
  initial begin
    #1 o = '0;
    o[-1:0] = 16'h9876;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "prop=00abcdef00000000 elem=0000112233000000",
            "prop2=77abcdef00000000",
            "prop3=ccdd000000001122",
            "fw=beef000000000000",
            "s=0000000dead010000",
            "aa=0000424200000000",
            "o=0000009876000000",
        ]
    );
}
