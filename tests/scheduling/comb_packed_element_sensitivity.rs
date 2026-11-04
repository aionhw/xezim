//! §9.2.2.2.1: an `always_comb` is sensitive to the longest static prefix of
//! each select it reads, excluding only the expressions it also writes. A
//! block that reads `ds[0]` and writes `ds[1]` of a packed vector (any shape
//! of select: bit, element, part select, non-constant index) must wake when
//! `ds[0]` is driven from outside, while a block that rewrites the element it
//! reads or the whole vector must still settle.

use xezim::simulate;

fn run(src: &str) -> (Vec<String>, u64) {
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs = sim.output.iter().map(|o| o.message.clone()).collect();
    (msgs, sim.settle_limit_report().0)
}

#[test]
fn always_comb_reading_one_packed_element_and_writing_another_wakes_on_its_driver() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] a;
  logic [1:0][7:0] ds;
  logic [7:0] sb;
  assign ds[0] = a;
  always_comb begin sb = ds[0] ^ 8'h01; ds[1] = sb; end
  initial begin
    a = 8'h01; #1 $display("ds1=%h", ds[1]);
    a = 8'h22; #1 $display("ds1=%h", ds[1]);
  end
endmodule
"#);
    assert_eq!(msgs, ["ds1=00", "ds1=23"]);
    assert_eq!(hits, 0);
}

#[test]
fn always_comb_rewriting_the_packed_element_it_reads_settles() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] di;
  logic [2:0][7:0] lo;
  logic [7:0] r1;
  logic [1:0][7:0] tw;
  assign tw[0] = di;
  always_comb begin tw[1] = '0; tw[1] = tw[0]; end
  wire [3:0] sa; assign sa = {sa[2:0], 1'b1};
  always_comb begin lo[0] = di ^ 8'h0f; lo[0] ^= 8'h01; end
  always_comb begin r1 = ~lo[0]; end
  assign lo[1] = r1 ^ 8'h3c;
  assign lo[2] = ~lo[1];
  initial begin
    di = 8'h10; #1 $display("lo2=%h tw1=%h sa=%b", lo[2], tw[1], sa);
    di = 8'h33; #1 $display("lo2=%h tw1=%h", lo[2], tw[1]);
  end
endmodule
"#);
    assert_eq!(msgs, ["lo2=22 tw1=10 sa=1111", "lo2=01 tw1=33"]);
    assert_eq!(hits, 0);
}

/// §9.2.2.2.1(b) excludes only the select expressions the block writes: a
/// block that reads an element it writes (`d3[1][0]`) beside one driven from
/// outside (`d3[0][1]`) stays sensitive to the vector, and a chain of blocks
/// on one vector settles in order.
#[test]
fn always_comb_reading_its_own_element_beside_an_outside_element_wakes() {
    let (msgs, hits) = run(r#"
module top;
  logic [2:0][7:0] ch;
  logic [1:0][1:0][7:0] d3;
  logic [7:0] src;
  always_comb ch[0] = src;
  always_comb ch[1] = ch[0] ^ 8'h55;
  always_comb ch[2] = ch[1] + 8'd3;
  assign d3[0][1] = src;
  always_comb begin d3[1][0] = d3[0][1] << 1; d3[1][1] = d3[1][0] | 8'h1; end
  initial begin
    src = 8'h0f; #1 $display("ch=%h d3=%h", ch, d3);
    src = 8'hf0; #1 $display("ch=%h d3=%h", ch, d3);
    src = 8'h01; #1 $display("ch=%h d3=%h", ch, d3);
  end
endmodule
"#);
    assert_eq!(
        msgs,
        [
            "ch=5d5a0f d3=1f1e0fxx",
            "ch=a8a5f0 d3=e1e0f0xx",
            "ch=575401 d3=030201xx"
        ]
    );
    assert_eq!(hits, 0);
}

/// §9.2.2.2.1: bit selects of a one-dimensional vector, constant part
/// selects and a read at a non-constant index (`st[i]`, whose longest static
/// prefix `st` is not what the block writes) all keep the vector in the
/// sensitivity.
#[test]
fn always_comb_bit_part_and_dynamic_selects_of_a_packed_vector_stay_sensitive() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] a, v, w;
  logic [3:0][7:0] st;
  assign v[0] = a[0];
  always_comb begin v[1] = v[0]; v[2] = v[1] ^ 1'b1; end
  assign w[3:0] = a[3:0];
  always_comb begin w[7:4] = w[3:0] + 4'd1; end
  int i = 0;
  assign st[0] = a;
  always_comb begin st[1] = st[i] + 8'd1; end
  initial begin
    a = 8'h00; #1 $display("v=%b w=%h st=%h", v, w, st);
    a = 8'h01; #1 $display("v=%b w=%h st=%h", v, w, st);
    a = 8'h0e; #1 $display("v=%b w=%h st=%h", v, w, st);
  end
endmodule
"#);
    assert_eq!(
        msgs,
        [
            "v=xxxxx100 w=10 st=xxxx0100",
            "v=xxxxx011 w=21 st=xxxx0201",
            "v=xxxxx100 w=fe st=xxxx0f0e"
        ]
    );
    assert_eq!(hits, 0);
}

/// §9.2.2.2.1, §11.5.1: a whole read beside a constant element write,
/// indexed part selects, element ranges of a two-dimensional vector, a part
/// select of one element, a three-bit vector and parameter indices.
#[test]
fn always_comb_part_select_forms_of_a_packed_vector_stay_sensitive() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] a;
  logic [7:0] x; logic [7:0] tmp;
  assign x[0] = a[0];
  always_comb begin tmp = x; x[3] = tmp[0]; end
  logic [7:0] w;
  assign w[0 +: 4] = a[3:0];
  always_comb w[7 -: 4] = w[0 +: 4] + 4'd1;
  logic [3:0][7:0] m;
  assign m[1:0] = {a, ~a};
  always_comb m[3:2] = m[1:0] + 16'd1;
  logic [1:0][7:0] n;
  assign n[0] = a;
  always_comb n[1][3:0] = n[0][7:4] ^ 4'hf;
  logic [2:0] c;
  assign c[0] = a[1];
  always_comb begin c[1] = c[0]; c[2] = c[1] & a[2]; end
  localparam int LO = 0, HI = 1;
  logic [1:0][7:0] pp;
  assign pp[LO] = a;
  always_comb pp[HI] = pp[LO] - 8'd1;
  initial begin
    a = 8'h00; #1 $display("x=%b w=%h m=%h n=%h c=%b pp=%h", x, w, m, n, c, pp);
    a = 8'h0f; #1 $display("x=%b w=%h m=%h n=%h c=%b pp=%h", x, w, m, n, c, pp);
    a = 8'hf6; #1 $display("x=%b w=%h m=%h n=%h c=%b pp=%h", x, w, m, n, c, pp);
  end
endmodule
"#);
    assert_eq!(
        msgs,
        [
            "x=xxxx0xx0 w=10 m=010000ff n=xf00 c=000 pp=ff00",
            "x=xxxx1xx1 w=0f m=0ff10ff0 n=xf0f c=111 pp=0e0f",
            "x=xxxx0xx0 w=76 m=f60af609 n=x0f6 c=111 pp=f5f6"
        ]
    );
    assert_eq!(hits, 0);
}

/// §9.2.2.2.1, §9.2.2.3, §9.4.2.2: an `always @*` and an `always_latch`
/// follow the same packed-element rule; neither re-queues on its own write.
#[test]
fn always_star_and_always_latch_follow_the_packed_element_rule() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] a;
  logic [7:0] v;
  assign v[0] = a[0];
  always @* begin v[1] = v[0]; v[2] = v[1] ^ 1'b1; end
  logic [1:0][7:0] lt;
  assign lt[0] = a;
  always_latch if (a[7]) lt[1] = lt[0] + 8'd2;
  initial begin
    a = 8'h00; #1 $display("v=%b lt=%h", v, lt);
    a = 8'h81; #1 $display("v=%b lt=%h", v, lt);
    a = 8'h36; #1 $display("v=%b lt=%h", v, lt);
  end
endmodule
"#);
    assert_eq!(
        msgs,
        [
            "v=xxxxx100 lt=xx00",
            "v=xxxxx011 lt=8381",
            "v=xxxxx100 lt=8336"
        ]
    );
    assert_eq!(hits, 0);
}
