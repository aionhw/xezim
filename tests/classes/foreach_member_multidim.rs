//! §12.7.3: procedural `foreach` over a FIXED multi-dim array PROPERTY
//! reached through a handle (`foreach (o.g2[i, j])`).
//!
//! The hier name resolved under the variable scope ("o.g2") while the
//! per-instance shape tables key the storage `<handle>#g2`, so every dims
//! lookup missed, none of the multi-var arms ran, and the loop collapsed to a
//! SINGLE iteration — silently. A 1-D member survived only via a later
//! element-key scan, which is why `foreach (o.g1[i])` always worked and the
//! collapse went unnoticed. The partial form (`foreach (o.g2[i])`, §12.7.3
//! first-dimension-only) collapsed the same way. Handle chains (`o.sub.g`)
//! resolve through the heap, hop by hop.
//!
//! Every expectation is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn foreach_over_handle_member_iterates_the_full_shape() {
    let o = out(r#"
module top;
  class inner_c;
    int g[2][3];
  endclass
  class c;
    int g1[4];
    int g2[3][3];
    int g3[2][2][2];
    inner_c sub;
    function new(); sub = new(); endfunction
    function int sum_this();
      int s = 0; foreach (this.g2[i, j]) s += this.g2[i][j]; return s;
    endfunction
  endclass
  initial begin
    c o = new();
    int n1, n2, n3, np, nc, s_rw, s_this;
    n1 = 0; foreach (o.g1[i]) n1++;
    n2 = 0; foreach (o.g2[i, j]) n2++;
    n3 = 0; foreach (o.g3[i, j, k]) n3++;
    np = 0; foreach (o.g2[i]) np++;              // partial: first dim only
    nc = 0; foreach (o.sub.g[i, j]) nc++;        // chained handle hop
    // Values through the handle: write in one loop, sum in another.
    foreach (o.g2[i, j]) o.g2[i][j] = 10*i + j;
    s_rw = 0; foreach (o.g2[i, j]) s_rw += o.g2[i][j];
    s_this = o.sum_this();
    $display("N=%0d/%0d/%0d P=%0d C=%0d RW=%0d TH=%0d",
             n1, n2, n3, np, nc, s_rw, s_this);
  end
endmodule
"#);
    assert!(
        o.contains("N=4/9/8 P=3 C=6 RW=99 TH=99"),
        "handle-member foreach shape wrong:\n{o}"
    );
}
