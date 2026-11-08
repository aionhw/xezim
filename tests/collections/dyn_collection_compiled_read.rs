//! IEEE 1800-2023 §7.5 / §7.10 / §7.4.6: an element read of a module-scope
//! dynamic array or queue in COMPILED code reads the live collection.
//!
//! The bytecode compiler addressed such a read through the collection's
//! placeholder `0..63` element cells (`array_first_id`), which are not the
//! collection: elements from index 64 on live in by-name storage, and a cell
//! past the live size keeps whatever it held before the collection shrank.
//! So a tight `for` loop `t += q[i]` over 65 or more elements read x at
//! index 64, and the 2-state sum collapsed to 0, while `foreach`, a single
//! read and a loop with a `$display` (all interpreted) were right. The same
//! read in an `always` block or a compiled function read 0, and a read past
//! `size()` returned the retired element instead of the default.
//!
//! Element stores already left the compiled path (`collection_store_denied`);
//! reads now do too. Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module top;
  int q[$];
  int d[];
  longint lq[$];
  logic [7:0] lg[$];
  int t[20];
  longint t5;
  int r1, r2, r3, r4;
  logic clk = 0;
  int w;
  function automatic int fsum(); int s = 0; for (int k = 0; k < 70; k++) s += q[k]; return s; endfunction
  always @(posedge clk) r3 <= q[w] + d[w];
  initial begin
    for (int i = 0; i < 100; i++) begin q.push_back(i); lq.push_back(i); lg.push_back(i[7:0]); end
    d = new[100]; foreach (d[i]) d[i] = i;
    t[0] = 0; for (int i = 0; i < 65; i++) t[0] += q[i];
    t5 = 0; for (int i = 0; i < 65; i++) t5 += lq[i];
    t[1] = 0; for (int i = 60; i < 66; i++) t[1] += q[i];
    t[2] = 0; for (int i = 0; i < 65; i++) t[2] += d[i];
    begin int i = 0; t[3] = 0; while (i < 65) begin t[3] += q[i]; i++; end end
    begin int i = 0; t[4] = 0; do begin t[4] += q[i]; i++; end while (i < 65); end
    t[5] = 0; for (int i = 0; i < 65; i++) t[5] -= q[i];
    t[6] = 0; for (int i = 0; i < 65; i++) t[6] |= q[i];
    t[7] = 0; for (int i = 0; i < 65; i++) t[7] ^= q[i];
    t[8] = 0; for (int i = 0; i < 70; i++) if (q[i] > 60) t[8]++;
    t[9] = 0; for (int i = 0; i < 65; i++) t[9] = t[9] + q[i];
    t[10] = 0; for (int i = 0; i < 100; i++) t[10] += q[i];
    t[11] = q[64];
    t[12] = 0; for (int i = 0; i < 65; i++) t[12] += q[99 - i];
    t[13] = 0; for (int i = 0; i < 100; i++) t[13] += lg[i];
    begin int i; i = 0; t[14] = 0; while (i < 70) begin t[14] += d[i]; i++; end end
    r1 = fsum();
    w = 80; #1 clk = 1; #1;
    $display("A %p %0d", t, t5);
    $display("B %0d %0d", r1, r3);
    // Reads past the live size read the element default, also compiled.
    q = '{1, 2, 3};
    q.delete();
    r1 = 0; for (int k = 0; k < 3; k++) r1 += q[k];
    q = '{5, 6, 7};
    void'(q.pop_front());
    r2 = 0; for (int k = 0; k < 3; k++) r2 += q[k];
    d = new[3]; d[0] = 1; d[1] = 2; d[2] = 3;
    d = new[1](d);
    r3 = 0; for (int k = 0; k < 3; k++) r3 += d[k];
    d.delete();
    r4 = 0; for (int k = 0; k < 3; k++) r4 += d[k];
    $display("C %0d %0d %0d %0d", r1, r2, r3, r4);
    $finish;
  end
endmodule
"#;

#[test]
fn compiled_collection_reads_see_the_live_collection() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "A '{2080, 375, 2080, 2080, 2080, -2080, 127, 64, 9, 2080, 4950, 64, 4355, 4950, \
         2415, 0, 0, 0, 0, 0} 2080",
        "B 2415 160",
        "C 0 13 1 0",
    ] {
        assert!(
            out.iter().any(|l| l.trim() == want),
            "missing {want:?} in {out:?}"
        );
    }
}
