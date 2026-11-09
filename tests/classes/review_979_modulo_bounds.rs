//! #979 review: §18.5.8.1, §11.4.4: a foreach modulo equality draws the
//! element from its congruence class, and the draw must also respect the
//! body's `<` / `<=` / `>` / `>=` bounds on the same element (written either
//! way round), or the relational repair undoes it and `randomize()` fails.
//! Expected values come from the reference simulator.

use xezim::simulate;

#[test]
fn modulo_draw_respects_relational_bounds() {
    let src = r#"
class A;
  rand int unsigned base[4];
  rand int unsigned sz[4];
  constraint c1 { foreach (sz[i]) sz[i] inside {256, 512}; }
  constraint c2 { foreach (base[i]) { (base[i] % sz[i]) == 0; base[i] < 4096; base[i] + sz[i] <= 4096; } }
endclass
class B;
  rand int v[4];
  constraint c { foreach (v[i]) { (v[i] % 10) == -3; v[i] > -50; -10 >= v[i]; } }
endclass
class Cc;
  rand bit [15:0] w[3];
  constraint c { foreach (w[i]) { (w[i] % 100) == 7; w[i] >= 1000; w[i] <= 1300; } }
endclass
module top;
  A a; B b; Cc c; int ok, bad;
  initial begin
    a = new; b = new; c = new;
    repeat (20) begin
      ok += a.randomize();
      foreach (a.base[i]) if (a.base[i] % a.sz[i] != 0 || a.base[i] >= 4096 || a.base[i] + a.sz[i] > 4096) bad++;
    end
    $display("A ok=%0d bad=%0d", ok, bad); ok = 0; bad = 0;
    repeat (20) begin
      ok += b.randomize();
      foreach (b.v[i]) if (b.v[i] % 10 != -3 || b.v[i] <= -50 || b.v[i] > -10) bad++;
    end
    $display("B ok=%0d bad=%0d", ok, bad); ok = 0; bad = 0;
    repeat (20) begin
      ok += c.randomize();
      foreach (c.w[i]) if (c.w[i] % 100 != 7 || c.w[i] < 1000 || c.w[i] > 1300) bad++;
    end
    $display("C ok=%0d bad=%0d", ok, bad);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(out, ["A ok=20 bad=0", "B ok=20 bad=0", "C ok=20 bad=0"]);
}
