//! IEEE 1800-2023 §5.7.1: "An unsized number that requires more than 32 bits
//! shall have at least the minimum width needed to properly represent the
//! value, including a sign bit if the number is signed." An unsized decimal
//! of 2^31 or more is therefore a wider signed number with its full value —
//! in a parameter, a procedural or continuous assignment, a function, a
//! class method, `$bits` and a constant expression alike (#1120). Before,
//! a parameter wrapped to 32 bits and a run-time literal lost its sign bit
//! (`2000000000000` read -199023255552). A `'d` number stays unsigned and
//! keeps the width of its digits, and assigning a wide value to a narrower
//! variable still truncates.

fn lines(src: &str, max_time: u64) -> Vec<String> {
    xezim::simulate(src, max_time)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with("Simulation"))
        .collect()
}

#[test]
fn reported_values_and_widths() {
    let src = r#"
`timescale 1ns/1ns
module top;
  localparam P = 2000000000000;
  longint x, y;
  assign y = 2000000000000;
  initial begin
    x = 2000000000000;
    #1 $display("param=%0d bits=%0d", P, $bits(P));
    $display("procedural=%0d continuous=%0d bits=%0d", x, y, $bits(2000000000000));
  end
endmodule
"#;
    assert_eq!(
        lines(src, 100),
        [
            "param=2000000000000 bits=42",
            "procedural=2000000000000 continuous=2000000000000 bits=42",
        ]
    );
}

#[test]
fn widths_from_two_to_the_31() {
    let src = r#"
`timescale 1ns/1ns
module child #(parameter int unsigned Q = 1, parameter R = 2) ();
  initial #2 $display("child Q=%0d R=%0d bitsR=%0d", Q, R, $bits(R));
endmodule
module top;
  localparam P = 3000000000;
  localparam P2 = 2147483648;
  localparam P3 = 2147483647;
  localparam [63:0] P4 = 4294967296 - 3000000000;
  localparam E = 'hffff_ffff_ffff_ffff == -1;
  int i;
  longint l, l2, l3;
  real r;
  logic [63:0] v;
  wire [63:0] w = 5000000000;
  child #(.Q(3000000000), .R(6000000000)) u();
  function automatic longint f(); return 3000000000 * 2; endfunction
  class C; longint p; function void set(); p = 7000000000 + 1; endfunction endclass
  C c;
  initial begin
    i = 3000000000;
    l = 3000000000;
    l2 = 3000000000 * 2;
    l3 = -2147483648;
    r = 3000000000;
    v = 'sd3000000000;
    c = new; c.set();
    $display("P=%0d %0d P2=%0d %0d P3=%0d %0d P4=%0d E=%0d", P, $bits(P), P2, $bits(P2), P3, $bits(P3), P4, E);
    $display("i=%0d l=%0d l2=%0d l3=%0d %0d r=%0.1f v=%0d", i, l, l2, l3, $bits(-2147483648), r, v);
    $display("f=%0d p=%0d bits=%0d %0d %0d", f(), c.p, $bits(3000000000), $bits('sd3000000000), $bits('d3000000000));
    $display("cmp=%0d %0d %0d", 3000000000 > 0, 3000000000 == 32'd3000000000, 4294967295 + 1);
    #1 $display("w=%0d", w);
    #2 $display("t=%0t", $time);
    #3000000000 $display("t2=%0t", $time);
  end
endmodule
"#;
    assert_eq!(
        lines(src, 4_000_000_000),
        [
            "P=3000000000 33 P2=2147483648 33 P3=2147483647 32 P4=1294967296 E=1",
            "i=-1294967296 l=3000000000 l2=6000000000 l3=-2147483648 33 r=3000000000.0 v=3000000000",
            "f=6000000000 p=7000000001 bits=33 33 32",
            // 4294967295 is 33 bits signed, so its self-determined sum with 1
            // is 33 bits signed as well, and 2^32 reads back negative there.
            "cmp=1 1 -4294967296",
            "w=5000000000",
            "child Q=3000000000 R=6000000000 bitsR=34",
            "t=3",
            "t2=3000000003",
        ]
    );
}
