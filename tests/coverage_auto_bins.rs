//! §19.5.1/§19.6 — coverage of AUTO-binned coverpoints and crosses must be the
//! fraction of auto bins hit, NOT 100% the moment anything is sampled. A 3-bit
//! coverpoint has 8 auto bins; a 2x2-bit cross has 16.

use xezim::simulate;

fn cov(src: &str) -> String {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn auto_bin_coverpoint_is_fraction_hit() {
    let o = cov(r#"
module t;
  bit [2:0] a;
  covergroup cg; cp: coverpoint a; endgroup
  cg c = new;
  initial begin
    a=0; c.sample(); a=3; c.sample(); a=7; c.sample();
    $display("COV=%0.2f", c.get_coverage());
    $finish;
  end
endmodule
"#);
    assert!(o.contains("COV=37.50"), "3 of 8 auto bins = 37.5%; got: {}", o);
}

#[test]
fn auto_cross_coverage_is_fraction_hit() {
    let o = cov(r#"
module t;
  bit [1:0] a, b;
  covergroup cg; ca: coverpoint a; cb: coverpoint b; axb: cross ca, cb; endgroup
  cg c = new;
  initial begin
    a=0;b=0; c.sample(); a=1;b=2; c.sample();
    // ca=2/4=50, cb=2/4=50, axb=2/16=12.5 -> mean 37.5
    $display("COV=%0.2f", c.get_coverage());
    $finish;
  end
endmodule
"#);
    assert!(o.contains("COV=37.50"), "cross must count 2 of 16 bins; got: {}", o);
}
