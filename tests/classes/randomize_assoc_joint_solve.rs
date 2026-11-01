//! §18.4/§18.5: a rand associative array keeps its keys and each element is
//! a random variable solved with the rest of the class. A class holding one
//! was kept out of the joint solver, so a constraint the per-variable trials
//! cannot repair (an array `sum()` tied to another variable by `==`) failed
//! every trial and randomize() returned 0.

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T "))
        .collect()
}

#[test]
fn enum_keyed_rand_assoc_joins_the_joint_solve() {
    let out = tagged(
        r#"
typedef enum bit [3:0] {
  C0 = 4'h0, C3 = 4'h3, C8 = 4'h8, C12 = 4'hc
} cause_t;
class cfg_c;
  rand int main_cnt;
  rand int sub_cnt[];
  rand bit flag[cause_t];
  constraint default_c {
    sub_cnt.size() == 5;
    main_cnt inside {[10 : 10000]};
    foreach (sub_cnt[i]) sub_cnt[i] inside {[10 : 10000]};
  }
  constraint sum_c { main_cnt + sub_cnt.sum() == 10000; }
  constraint flag_c {
    flag[C12] == 1'b0;
    foreach (flag[i]) {
      if (!(i inside {C0, C3})) flag[i] == 0;
    }
  }
  function new();
    cause_t c = c.first;
    do begin
      flag[c] = 1;
      c = c.next;
    end while (c != c.first);
  endfunction
endclass
module top;
  initial begin
    cfg_c c = new();
    int good = 1;
    for (int t = 0; t < 4; t++) begin
      int s = 0;
      if (!c.randomize()) good = 0;
      foreach (c.sub_cnt[i]) begin
        s += c.sub_cnt[i];
        if (!(c.sub_cnt[i] inside {[10:10000]})) good = 0;
      end
      if (!(c.sub_cnt.size() == 5 && c.main_cnt inside {[10:10000]} &&
            c.main_cnt + s == 10000 && c.flag.num() == 4 &&
            c.flag[C12] == 0 && c.flag[C8] == 0)) good = 0;
    end
    $display("T good=%0d", good);
  end
endmodule
"#,
    );
    assert_eq!(out, ["T good=1"]);
}
