//! IEEE 1800-2017 §18.4.2: a `randc` variable wider than 16 bits cycles
//! too (it was drawn as `rand`, with repeats, #699, reported by
//! Ganesh-TNL). A 32-bit `randc` held to 70,000 values draws 1,000 distinct
//! values (independent draws would repeat about 7 times), one held to 100
//! values covers them all before repeating, and an unconstrained one draws
//! distinct values. Counts match the expected values.
use xezim::simulate;

#[test]
fn wide_randc_cycles() {
    let out: Vec<String> = simulate(
        r#"class rw;
  randc bit [31:0] v;
  constraint c { v inside {[0:69999]}; }
endclass
class rn;
  randc bit [31:0] v;
  constraint c { v inside {[100:199]}; }
endclass
class rf;
  randc bit [31:0] v;
endclass
module t;
  bit seen[int];
  initial begin
    rw a = new; rn b = new; rf f = new;
    int dup = 0, out = 0, dupn = 0, outn = 0, dupf = 0;
    bit [31:0] fv[$];
    for (int i = 0; i < 1000; i++) begin
      void'(a.randomize());
      if (a.v > 69999) out++;
      if (seen.exists(a.v)) dup++; else seen[a.v] = 1;
    end
    seen.delete();
    for (int i = 0; i < 100; i++) begin
      void'(b.randomize());
      if (b.v < 100 || b.v > 199) outn++;
      if (seen.exists(b.v)) dupn++; else seen[b.v] = 1;
    end
    void'(b.randomize());
    seen.delete();
    for (int i = 0; i < 500; i++) begin
      void'(f.randomize());
      if (seen.exists(f.v)) dupf++; else seen[f.v] = 1;
    end
    $display("R %0d %0d %0d %0d %0d %0d", dup, out, dupn, outn, seen.num(), dupf);
  end
endmodule
"#,
        10_000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with("R "))
    .collect();
    assert_eq!(out, ["R 0 0 0 0 500 0"]);
}
