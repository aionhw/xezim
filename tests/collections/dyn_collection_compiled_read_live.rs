//! IEEE 1800-2023 §7.5 / §7.10 / §7.4.6: compiled code reads a dynamic array
//! or queue element from the live collection, as the interpreter does.
//!
//! A compiled element read of a design-scope dynamic array or queue is one
//! `LoadDynElem` instruction: an index inside both `size()` and the
//! collection's placeholder element cells reads its cell; anything else (past
//! the cells, past `size()`, an x or negative index) takes the interpreter's
//! own element read, so by-name storage past index 63, the element type's
//! default (x for a 4-state element, 0 for a 2-state one, null for a handle)
//! and signedness all come out as they do interpreted. Each loop below is a
//! compiled process loop; the cases straddle index 63 and the live size
//! after shrinks, inserts, deletes and a reverse.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
class c_t; int v; function new(int x); v = x; endfunction endclass
module top;
  logic [7:0] lq[$];
  byte sb[$];
  int d[];
  int q[$];
  c_t hq[$];
  logic [7:0] seen[8];
  int r[16];
  integer xi;
  int neg;
  initial begin
    for (int i = 0; i < 70; i++) begin lq.push_back(8'h80 + i[7:0]); sb.push_back(-i); end
    // 4-state element past the size reads x; the cells of a shrink stay unread.
    repeat (66) void'(lq.pop_back());
    for (int k = 0; k < 8; k++) seen[k] = lq[k];
    // signed 2-state elements, across the cell range
    r[0] = 0; for (int k = 60; k < 70; k++) r[0] += sb[k];
    // a dynamic array sized past the cells reads its default
    d = new[200]; for (int k = 190; k < 200; k++) r[1] += d[k] + 1;
    d[150] = 7; r[2] = 0; for (int k = 140; k < 160; k++) r[2] += d[k];
    // insert / delete(i) / reverse move elements across index 63
    for (int i = 0; i < 80; i++) q.push_back(i);
    q.insert(10, 1000); q.delete(70); q.reverse();
    r[3] = 0; for (int k = 0; k < 80; k++) r[3] += q[k] * (k + 1);
    // an x or negative index reads the default
    neg = -1; r[4] = 0; for (int k = 0; k < 3; k++) r[4] += q[neg] + q[xi];
    // handle queue: null past the size
    for (int i = 0; i < 66; i++) begin c_t h; h = new(i); hq.push_back(h); end
    r[5] = 0; for (int k = 0; k < 70; k++) if (hq[k] == null) r[5]++;
    r[6] = 0; for (int k = 0; k < 66; k++) r[6] += hq[k].v;
    $display("A %h %h %h %h %h", seen[0], seen[3], seen[4], seen[5], seen[7]);
    $display("B %p", r);
    $finish;
  end
endmodule
"#;

#[test]
fn compiled_reads_match_the_interpreter() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "A 80 83 xx xx xx",
        "B '{-645, 10, 7, 152260, 0, 4, 2145, 0, 0, 0, 0, 0, 0, 0, 0, 0}",
    ] {
        assert!(
            out.iter().any(|l| l.trim() == want),
            "missing {want:?} in {out:?}"
        );
    }
}
