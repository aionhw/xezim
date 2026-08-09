//! §7.8/§7.9 — associative-array key semantics and class-handle elements:
//! full-range unsigned 64-bit keys in first/last/exists, next() across
//! signed keys, and write-through on a handle stored in an assoc array.
//! All reference-validated (audit round I2-I4).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn assoc_u64_full_range_keys() {
    let src = r#"
module tb;
  int ba[bit [63:0]];
  bit [63:0] kf, kl;
  int ex;
  initial begin
    ba[10] = 1;
    ba[64'hFFFF_FFFF_FFFF_FFFF] = 2;
    void'(ba.first(kf));
    void'(ba.last(kl));
    ex = ba.exists(64'hFFFF_FFFF_FFFF_FFFF);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "kf"), 10, "first is the small key, not lexicographic");
    assert_eq!(u(&sim, "kl"), u64::MAX, "last returns the key value, not its string bytes");
    assert_eq!(u(&sim, "ex"), 1);
}

#[test]
fn assoc_next_across_signed_keys() {
    let src = r#"
module tb;
  int aa[int];
  int k, ok;
  initial begin
    aa[-3] = 1; aa[5] = 2;
    void'(aa.first(k));
    ok = aa.next(k);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "next finds the current negative key");
    assert_eq!(u(&sim, "k"), 5, "next steps -3 -> 5");
}

#[test]
fn assoc_class_handle_write_through() {
    let src = r#"
class Item;
  int v = 1;
endclass
module tb;
  Item byname[string];
  int direct_v, elem_v;
  initial begin
    Item i1 = new;
    byname["a"] = i1;
    byname["a"].v = 11; // write through the STORED handle
    direct_v = i1.v;
    elem_v = byname["a"].v;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "direct_v"), 11, "element aliases the object, not a copy");
    assert_eq!(u(&sim, "elem_v"), 11);
}
