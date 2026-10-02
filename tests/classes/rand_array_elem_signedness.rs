//! §6.11 / §18.4: a value randomize() draws for an element of a rand array
//! of a signed type (`rand int q[2]`, `rand byte b[]`) is signed. The
//! solvers draw and repair elements as bit patterns: a fixed array's
//! baseline and every collection write stored them unsigned, so a negative
//! pick (`q[i] inside {[-3:-1]}`) read back as 4294967293 and compared
//! `< 0` as false. An unsigned element type stays unsigned.

use xezim::simulate;

const SRC: &str = r#"
class fixed_c;  rand int  f[2];    constraint k { foreach (f[i]) f[i] inside {[-3:-1]}; } endclass
class nd_c;     rand int  f[2][2]; constraint k { foreach (f[i, j]) f[i][j] inside {[-3:-1]}; } endclass
class byte_c;   rand byte f[2];    constraint k { foreach (f[i]) f[i] inside {[-3:-1]}; } endclass
class dyn_c;    rand int  f[];     constraint k { f.size() == 2; foreach (f[i]) f[i] inside {[-3:-1]}; } endclass
class queue_c;  rand int  f[$];    constraint k { f.size() == 2; foreach (f[i]) f[i] inside {[-3:-1]}; } endclass
class rel_c;    rand int  f[2];    constraint k { foreach (f[i]) { f[i] < 0; f[i] > -100; f[i] % 2 == 0; } } endclass
class order_c;  rand int  f[2];    constraint k { foreach (f[i]) f[i] inside {[-10:10]}; f[0] < f[1]; f[0] < 0; } endclass
class ubit_c;   rand bit [7:0] f[2]; constraint k { foreach (f[i]) f[i] inside {[250:255]}; } endclass
module top;
  int fails;
  initial begin
    fixed_c a = new(); nd_c b = new(); byte_c c = new(); dyn_c d = new();
    queue_c e = new(); rel_c g = new(); order_c h = new(); ubit_c u = new();
    for (int n = 0; n < 20; n++) begin
      if (!a.randomize()) fails++; foreach (a.f[i]) if (!(a.f[i] < 0 && a.f[i] >= -3)) fails++;
      if (!b.randomize()) fails++; foreach (b.f[i, j]) if (!(b.f[i][j] < 0 && b.f[i][j] >= -3)) fails++;
      if (!c.randomize()) fails++; foreach (c.f[i]) if (!(c.f[i] < 0 && c.f[i] >= -3)) fails++;
      if (!d.randomize()) fails++; foreach (d.f[i]) if (!(d.f[i] < 0 && d.f[i] >= -3)) fails++;
      if (!e.randomize()) fails++; foreach (e.f[i]) if (!(e.f[i] < 0 && e.f[i] >= -3)) fails++;
      if (!g.randomize()) fails++; foreach (g.f[i]) if (!(g.f[i] < 0 && g.f[i] > -100 && g.f[i] % 2 == 0)) fails++;
      if (!h.randomize() || !(h.f[0] < 0 && h.f[0] < h.f[1] && h.f[0] >= -10 && h.f[1] <= 10)) fails++;
      if (!u.randomize()) fails++; foreach (u.f[i]) if (!(u.f[i] >= 250)) fails++;
    end
    $display("fails=%0d a=%0d d=%0d u=%0d", fails, a.f[0] < 0, d.f[0] < 0, u.f[0] > 0);
  end
endmodule
"#;

#[test]
fn rand_signed_array_elements_keep_their_sign() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(out, ["fails=0 a=1 d=1 u=1"], "{out:?}");
}
