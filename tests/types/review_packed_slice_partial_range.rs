//! §11.5.1: a part-select after an index into a multi-dimensional packed
//! array selects whole elements of the next dimension, and when its run-time
//! base runs partly past that dimension a write affects only the elements
//! inside it and a read returns x for the elements outside it. A wholly
//! out-of-range write has no effect.
//!
//! The compiled stores (#1372) placed these; the interpreter, which runs an
//! `initial` block, sent the select to its generic part-select store once
//! the declaration walk declined it: `a[0][k +: 2] = 16'hbbaa` with `k = 5`
//! wrote nothing, `b[1][k0 -: 2]` with `k0 = 0` wrote one bit, and the reads
//! came back as bits. Every expected value matches the reference simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn interpreted_part_select_past_its_dimension_touches_only_elements_inside() {
    let src = r#"
module t;
  logic [3:0][5:0][7:0] a, b, e, f;
  logic [3:0][0:5][7:0] g;
  bit [1:0][3:0][7:0] tb;
  logic [2:0] k = 3'd5, k0 = 3'd0, k7 = 3'd7;
  logic [15:0] r1, r2, r3;
  logic [3:0][5:0][7:0] h;
  logic clk = 0;
  always_ff @(posedge clk) begin
    h <= '0;
    h[2][k +: 2] <= 16'h1122;
  end
  initial begin
    a = '0; b = '0; e = '0; f = '0; g = '0; tb = '0;
    #1;
    a[0][k +: 2] = 16'hbbaa;
    b[1][k0 -: 2] = 16'hccdd;
    e[2][k7 +: 2] = 16'heeee;
    f[3][0][k +: 4] = 4'hf;
    g[1][k +: 2] = 16'h7788;
    tb[1][k0 + 3'd3 +: 2] = 16'h5544;
    r1 = a[0][k +: 2];
    r2 = b[1][k0 -: 2];
    r3 = tb[1][k0 + 3'd3 +: 2];
    $display("T|a=%h", a);
    $display("T|b=%h", b);
    $display("T|e=%h", e);
    $display("T|f=%h", f);
    $display("T|g=%h", g);
    $display("T|tb=%h", tb);
    $display("T|r1=%h r2=%h r3=%h", r1, r2, r3);
    #1 clk = 1;
    #1 $display("T|h=%h", h);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|a=000000000000000000000000000000000000aa0000000000",
            "T|b=0000000000000000000000000000000000cc000000000000",
            "T|e=000000000000000000000000000000000000000000000000",
            "T|f=0000000000e0000000000000000000000000000000000000",
            "T|g=000000000000000000000000000000000077000000000000",
            "T|tb=4400000000000000",
            "T|r1=xxaa r2=ccxx r3=0044",
            "T|h=000000000000220000000000000000000000000000000000",
        ]
    );
}

/// §7.2.1, §7.4.3: a slice below an index into a multi-dimensional packed
/// struct member (`s.m[1][2:1]`) is whole elements too. The interpreter got
/// the member as one hierarchical name, `s.m`, which has no declaration of
/// its own, and read and wrote it as 2 bits.
#[test]
fn interpreted_slice_of_packed_struct_member_selects_elements() {
    let src = r#"
module t;
  typedef struct packed { logic [3:0] tag; logic [2:0][3:0][7:0] m; } s_t;
  s_t s;
  logic [2:0] k0 = 3'd0;
  logic [31:0] r;
  initial begin
    s = '0;
    #1;
    s.m[2][2:1] = 16'h1234;
    s.m[1][k0 -: 2] = 16'h99aa;
    r = s.m[2][2:1];
    $display("T|s=%h r=%h", s, r);
    $display("T|bits=%0d %0d", $bits(s.m[1][2:1]), $bits(s.m[1][1 +: 2]));
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        ["T|s=0001234000000009900000000 r=00001234", "T|bits=16 16"]
    );
}
