//! §7.12.3, §18.5.8.2, §18.4: array reductions in constraints.
//!
//! `and()`, `or()`, `xor()` and `product()` equalities were accepted
//! without being checked, so `randomize()` returned 1 with the constraint
//! violated; they are now solved by constructing the elements and checked.
//! `sum()` and `or()` with `with (item.code)` over packed-struct elements
//! read 0 for every element and made the set unsatisfiable; the result wraps
//! at the field's width (`F + B == 10` over 4 bits). An associative array is
//! never resized by randomization, so `aa.size() == 3` on an empty one must
//! fail rather than pass. Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
typedef struct packed { bit valid; bit [7:0] mask; bit [3:0] code; } beat_t;
class R;
  rand bit [15:0] wstrb[8];
  rand bit [3:0]  lane_en[4];
  rand bit [7:0]  parity[4];
  rand bit [7:0]  beat_cnt[4];
  rand bit [3:0]  a[];
  rand bit [3:0]  q[$];
  rand bit [7:0]  pr[3];
  constraint c1 { wstrb.and() == 16'hFFFF; lane_en.or() == 4'h0; parity.xor() == 8'h00; beat_cnt.product() == 0; }
  constraint c2 { a.size() == 4; a.and() == 4'hF; q.size() == 4; q.or() == 4'h0; pr.product() == 8'd15; }
endclass
class P; rand beat_t pool[4]; constraint c { pool.sum() with (item.code) == 10; pool.or() with (item.code) == 4'hF; } endclass
class PD; rand beat_t pool[]; constraint s { pool.size() == 4; } constraint c { pool.sum() with (item.code) == 10; } endclass
class AS; rand bit [3:0] aa[int]; constraint c { aa.size() == 3; } endclass
module tb;
  initial begin
    R r = new; P p = new; PD pd = new; AS as = new; int ok, bad = 0, sum, o;
    repeat (50) begin
      ok = r.randomize();
      if (!ok || r.wstrb.and() != 16'hFFFF || r.lane_en.or() != 0 || r.parity.xor() != 0 || r.beat_cnt.product() != 0
          || r.a.and() != 4'hF || r.q.or() != 0 || r.pr.product() != 15) bad++;
    end
    $display("T|H0 bad=%0d", bad);
    bad = 0;
    repeat (50) begin
      ok = p.randomize(); sum = 0; o = 0;
      foreach (p.pool[i]) begin sum += p.pool[i].code; o |= p.pool[i].code; end
      if (!ok || sum[3:0] != 10 || o != 15) bad++;
      ok = pd.randomize(); sum = 0;
      foreach (pd.pool[i]) sum += pd.pool[i].code;
      if (!ok || pd.pool.size() != 4 || sum[3:0] != 10) bad++;
    end
    $display("T|H1 bad=%0d", bad);
    ok = as.randomize();
    $display("T|H2 ok=%0d size=%0d", ok, as.aa.size());
  end
endmodule
"#;

#[test]
fn reduction_constraints_are_solved_and_checked() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|H0 bad=0", "T|H1 bad=0", "T|H2 ok=0 size=0"]);
}
