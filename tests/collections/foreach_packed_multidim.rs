//! §12.7.3: `foreach (a[i, j])` over a PURELY packed multi-dimensional array
//! maps the loop variables onto the packed dimensions left to right
//! (declared dimensions first, then a typedef's own). Without any unpacked
//! dimension the executor used to iterate one packed dimension and leave the
//! other variables x: `u7_t [4:0][1:0]` gave 5 iterations instead of 10 and a
//! `bit [6:0][4:0][1:0]` gave 7 instead of 35. Reference-verified counts.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn foreach_two_vars_over_packed_only_array_iterates_both_dimensions() {
    let msgs = messages(
        r#"
typedef bit [6:0] u7_t;
module tb;
  u7_t [4:0][1:0] a;
  bit [6:0][4:0][1:0] b;
  int n1 = 0, n2 = 0; string first_a = "", first_b = "";
  initial begin
    foreach (a[i, j]) begin n1++; if (n1 == 1) first_a = $sformatf("%0d,%0d", i, j); end
    foreach (b[i, j]) begin n2++; if (n2 == 1) first_b = $sformatf("%0d,%0d", i, j); end
    $display("N %0d %0d %s %s", n1, n2, first_a, first_b);
    $finish;
  end
endmodule
"#,
    );
    assert!(msgs.iter().any(|m| m == "N 10 35 4,1 6,4"), "got {msgs:?}");
}

#[test]
fn foreach_over_packed_array_elements_reads_each_element() {
    let msgs = messages(
        r#"
typedef bit [6:0] u7_t;
module tb;
  u7_t [4:0][1:0] a;
  int sum = 0, cnt = 0;
  initial begin
    foreach (a[i, j]) a[i][j] = i * 10 + j;
    foreach (a[i, j]) begin sum += a[i][j]; cnt++; end
    $display("S %0d %0d", cnt, sum);
    $finish;
  end
endmodule
"#,
    );
    // 10 elements; sum of i*10+j over i in 0..4, j in 0..1 = 200 + 5 = 205
    assert!(msgs.iter().any(|m| m == "S 10 205"), "got {msgs:?}");
}
