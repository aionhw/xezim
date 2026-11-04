//! §18.5.4, §18.5.7: `dist` weights on collection elements, and on an item
//! whose branch carries more constraints, for an object randomized again
//! and for a fresh object per call.
//!
//! A dist on an element of a FIXED array was taken as a plain `inside`, and
//! a dist on a FIELD of a dynamic array's struct element had no draw, so a
//! 40-weight item came out at the uniform baseline (about 0 in 1000). A 1 in
//! 10 item whose branch adds hard constraints came out 1 in 30 or less: a
//! failed trial used up its dealt item, and the joint solver that ran after
//! it repaired the trial toward the other item. A fresh object's first draw
//! was always the heaviest item, so it came out 0 times in 1000. The
//! reference simulator gives 399, 393, 90, 91, 99 and 99 for this source.

use xezim::simulate;

const SRC: &str = r#"
typedef struct packed { logic valid; logic [127:0] data; logic [15:0] mask; logic tail; } wpkt_t;
typedef struct packed {
   logic [7:0]  len;
   logic [2:0]  sz;
   logic [1:0]  kind;
} g_hdr_t;
class g_elemfix_c;
   rand bit [15:0] s[4];
   constraint c { foreach (s[i]) s[i] dist {65535 :/ 40, [0:65534] :/ 60}; }
endclass
class g_fielddyn_c;
   rand bit [7:0]  len;
   rand wpkt_t     p[];
   constraint c1 { p.size() == len + 1; }
   constraint c2 { foreach (p[i]) { p[i].valid == 1;
                   p[i].mask dist {65535 :/ 40, [0:65534] :/ 60}; } }
endclass
class g_wrapdyn_c;
   rand g_hdr_t     h;
   rand bit [13:0]  tot;
   rand bit [7:0]   pad[];
   constraint c1 { h.kind dist {1 := 9, 2 := 1}; }
   constraint c2 { if (h.kind == 2) { h.sz == 4; tot inside {32, 64, 128, 256}; } }
   constraint c3 { tot == ((h.len + 1) << h.sz); }
   constraint c4 { h.len <= 7; }
   constraint c5 { pad.size() == h.len + 1; }
endclass
class g_pin_c;
  rand bit [1:0] kind;
  rand bit [5:0] len;
  rand bit [7:0] pad[];
  constraint c1 { kind dist {1 := 9, 2 := 1}; }
  constraint c2 { if (kind == 2) ((len * 3) % 64) == 47; }
  constraint c3 { pad.size() == len[1:0] + 1; }
endclass
module tb;
  initial begin
    g_fielddyn_c g3 = new;
    g_wrapdyn_c g6 = new;
    g_pin_c pn = new;
    int n1 = 0, t1 = 0, n3 = 0, t3 = 0, n6 = 0, n6f = 0, np = 0, npf = 0, f = 0;
    for (int k = 0; k < 300; k++) begin
      automatic g_elemfix_c g1 = new();
      if (!g1.randomize()) f++;
      foreach (g1.s[i]) begin t1++; if (g1.s[i] == 65535) n1++; end
    end
    repeat (40) begin
      if (!g3.randomize()) f++;
      foreach (g3.p[i]) begin t3++; if (g3.p[i].mask == 65535) n3++; if (g3.p[i].valid !== 1) f++; end
    end
    repeat (1000) begin
      if (!g6.randomize()) f++;
      if (g6.h.kind == 2) n6++;
      if (!pn.randomize()) f++;
      if (pn.kind == 2) begin np++; if (((pn.len * 3) % 64) != 47) f++; end
    end
    for (int k = 0; k < 1000; k++) begin
      automatic g_wrapdyn_c w = new();
      automatic g_pin_c p = new();
      if (!w.randomize()) f++;
      if (w.h.kind == 2) n6f++;
      if (!p.randomize()) f++;
      if (p.kind == 2) npf++;
    end
    $display("T| fails=%0d g1=%0d g3=%0d g6=%0d g6f=%0d pin=%0d pinf=%0d", f, n1 * 1000 / t1, n3 * 1000 / t3, n6, n6f, np, npf);
  end
endmodule
"#;

#[test]
fn element_and_branch_dists_keep_their_weights() {
    let sim = simulate(SRC, 100).expect("simulate");
    let line = sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .find(|m| m.starts_with("T|"))
        .expect("result line");
    let get = |k: &str| -> i64 {
        line.split_whitespace()
            .find_map(|t| t.strip_prefix(k)?.strip_prefix('=')?.parse().ok())
            .unwrap_or_else(|| panic!("{k} missing from {line}"))
    };
    assert_eq!(get("fails"), 0, "{line}");
    // About four standard deviations around 400, 400 (permille) and 100 in
    // 1000 calls.
    assert!((340..=460).contains(&get("g1")), "{line}");
    assert!((350..=450).contains(&get("g3")), "{line}");
    for k in ["g6", "g6f", "pin", "pinf"] {
        assert!((62..=138).contains(&get(k)), "{k}: {line}");
    }
}
