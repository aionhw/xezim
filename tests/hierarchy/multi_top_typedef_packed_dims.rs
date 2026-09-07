//! A typedef'd packed multi-dimensional array (`u7_t [4:0][1:0] a`) declared
//! in an INSTANTIATED module — which every top of a multi-top design is,
//! under the synthetic wrapper — had no packed geometry recorded: the
//! instance path asked only for the declaration's own dimensions, which a
//! type reference does not carry, so `foreach (a[i, j])` iterated the 70
//! bits instead of the 10 elements while the same module run as the selected
//! top was right. Declared dims are now chained with the typedef's for
//! instance variables, ports and nets. Reference-verified counts.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn typedef_packed_dims_survive_the_multi_top_wrapper() {
    let msgs = messages(
        "typedef bit [6:0] u7_t;
module other_top; initial begin end endmodule
module leafm(input u7_t [4:0][1:0] p, output int n, output int e21);
  int k;
  always @* begin k = 0; foreach (p[i, j]) k++; n = k; e21 = p[2][1]; end
endmodule
module tb #(parameter R = 5, C = 2);
  u7_t [4:0][1:0] dc;
  u7_t [R-1:0][C-1:0] dp;
  u7_t [2:0][4:0][1:0] d3;
  u7_t [4:0][1:0] arr [3];
  int nc = 0, np = 0, n3 = 0, na = 0, nl, e21;
  leafm u(.p(dc), .n(nl), .e21(e21));
  initial begin
    dc = '0; dc[2][1] = 77;
    foreach (dc[i, j]) nc++;
    foreach (dp[i, j]) np++;
    foreach (d3[a, b, c]) n3++;
    foreach (arr[x, i, j]) na++;
    #1 $display(\"MT nc=%0d np=%0d n3=%0d na=%0d leaf_n=%0d leaf_e21=%0d rd=%0d size1=%0d\",
                nc, np, n3, na, nl, e21, dc[2][1], $size(dc, 1));
    $finish;
  end
endmodule
",
    );
    let want = "MT nc=10 np=10 n3=30 na=30 leaf_n=10 leaf_e21=77 rd=77 size1=5";
    assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
}
