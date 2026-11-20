//! §11.4.11, Table 7-1: an ambiguous real conditional keeps equal arms
//! and returns 0.0 for unequal arms.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn real_conditional_with_ambiguous_condition() {
    let o = out(r#"
module t;
  localparam real P = 1'bx ? 5.0 : 6.0;
  reg sel; real a = 1000.0, mout = 2.0, r;
  initial begin
    $display("P=%g", P);
    sel = 1'bz; r = sel ? a : 1000; $display("Z=%g", r);
    r = 2'b1x ? a : a*mout; $display("M=%g", r);
    sel = 1'bx; r = sel ? a : a*mout; $display("X=%g", r);   // unequal arms
    sel = 1'b1; r = sel ? a : a*mout; $display("T=%g", r);   // 1000
    sel = 1'b0; r = sel ? a : a*mout; $display("F=%g", r);   // 2000
    $finish;
  end
endmodule
"#);
    for expected in ["P=0", "X=0", "Z=1000", "M=1000"] {
        assert!(o.lines().any(|line| line == expected), "{o}");
    }
    assert!(
        o.contains("T=1000") && o.contains("F=2000"),
        "known selects still correct; got: {}",
        o
    );
}

/// An integral conditional with an X condition still does the per-bit merge.
#[test]
fn integral_conditional_with_x_still_merges() {
    let o = out(r#"
module t;
  reg sel; reg [3:0] a=4'b1100, b=4'b1010, r;
  initial begin sel=1'bx; r = sel ? a : b; $display("R=%b", r); $finish; end
endmodule
"#);
    assert!(
        o.contains("R=1xx0"),
        "integral x-select keeps per-bit merge; got: {}",
        o
    );
}
