//! §7.8 — associative array OF STRUCTS: registration, string-keyed element
//! naming, num()/exists(), and foreach key binding. Reference-validated
//! (audit round I14).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn string_keyed_assoc_of_struct() {
    let src = r#"
module tb;
  typedef struct { int id; } S;
  S sa[string];
  S tmp;
  int n, ex, fid;
  string fkey;
  initial begin
    tmp.id = 7;
    sa["k"] = tmp;
    n = sa.num();
    ex = sa.exists("k");
    foreach (sa[k]) begin
      fkey = k;
      fid = sa[k].id;
    end
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "n"), 1, "num() sees the struct element");
    assert_eq!(u(&sim, "ex"), 1, "exists() sees member-wise leaves");
    assert_eq!(u(&sim, "fid"), 7, "element member reads through the loop key");
    let fkey = sim
        .get_signal("fkey")
        .or_else(|| sim.get_signal("tb.fkey"))
        .expect("fkey");
    assert_eq!(fkey.to_sv_string(), "k", "foreach binds the string key");
}
