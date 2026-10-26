//! §20.9: `$countones` of a concatenation of rand variables in a constraint
//! counts the ones of every part. With the count fixed (`n == 1`, `n == 0`)
//! the constraints are satisfiable, so randomize() succeeds (§18.3).

use xezim::simulate;

const SRC: &str = r#"
class flags_c;
  rand bit a, b, d;
  rand bit [14:0] f;
  rand bit e;
  rand int n;
  constraint k {
    n inside {[0:31]};
    $countones({a, b, d, f, e}) == n;
  }
  constraint s { soft n == 1; }
  function int ok();
    return $countones({a, b, d, f, e}) == n;
  endfunction
endclass

module top;
  initial begin
    flags_c x = new;
    int ok[4];
    for (int i = 0; i < 20; i++) begin
      if (x.randomize() with { n == 1; e == 1; } && x.ok() && x.e && x.f == 0) ok[0]++;
      if (x.randomize() with { n == 0; } && x.ok() && {x.a, x.b, x.d, x.f, x.e} == 0) ok[1]++;
      if (x.randomize() with { n inside {[1:5]}; } && x.ok()) ok[2]++;
      if (x.randomize() && x.ok() && x.n == 1) ok[3]++;
    end
    $display("ok=%0d %0d %0d %0d", ok[0], ok[1], ok[2], ok[3]);
  end
endmodule
"#;

#[test]
fn countones_of_concatenation_meets_a_fixed_count() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("ok="))
        .collect();
    assert_eq!(got, ["ok=20 20 20 20"]);
}
