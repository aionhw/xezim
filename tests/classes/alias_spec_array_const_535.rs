//! IEEE 1800-2023 §8.25 / §6.18: a typedef of a specialization carries its
//! arguments, so `Alias::T` with `typedef C#(5) Alias` is the five-element
//! array, not the default two. The whole-array resolver dropped the
//! arguments the prefix lookup returned (#535, reported by akashlevy).

use xezim::simulate;

#[test]
fn typedef_of_a_specialization_sizes_its_array_constant() {
    let src = r#"
class C #(int N = 2);
  localparam int T[N] = '{default: 3};
endclass
typedef C#(5) Alias;
typedef C#(.N(4)) Named;
module t;
  int x[5];
  initial begin
    int n = 0;
    $display("size=%0d size2=%0d named=%0d", $size(Alias::T), $size(C#(5)::T), $size(Named::T));
    foreach (Alias::T[i]) n++;
    x = Alias::T;
    $display("n=%0d x4=%0d", n, x[4]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"size=5 size2=5 named=4"), "{msgs:?}");
    assert!(msgs.contains(&"n=5 x4=3"), "{msgs:?}");
}
