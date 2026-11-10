//! #940 review: the `exists()` fast path for a missing flat key is only
//! sound when the element is a scalar. §7.8, §7.9.3: an element that is an
//! array itself (a fixed-size row `int x[string][3]`, or a typedef with
//! unpacked dimensions `typedef int a2_t[2]; a2_t f[string]`) is stored
//! under `f["b"][1]`, and `f.exists("b")` is 1. Expected values come from
//! the reference simulator.

use xezim::simulate;

fn lines(src: &str, prefix: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with(prefix))
        .collect()
}

const SRC: &str = r#"
module top;
  typedef int a2_t[2];
  typedef a2_t a2b_t;
  a2_t f[string];
  a2b_t g[string];
  int x[string][3];
  int s[string];
  class K;
    a2_t cf[string];
    int cx[string][2];
  endclass
  initial begin
    K k = new;
    f["b"][1] = 7;
    g["c"][0] = 5;
    x["k"][2] = 9;
    s["s"] = 1;
    k.cf["d"][1] = 3;
    k.cx["e"][0] = 4;
    $display("E f=%0d g=%0d x=%0d s=%0d cf=%0d cx=%0d", f.exists("b"), g.exists("c"),
             x.exists("k"), s.exists("s"), k.cf.exists("d"), k.cx.exists("e"));
    $display("E miss f=%0d x=%0d s=%0d cf=%0d", f.exists("zz"), x.exists("zz"), s.exists("zz"),
             k.cf.exists("zz"));
  end
endmodule
"#;

#[test]
fn exists_finds_rows_of_fixed_and_typedef_arrays() {
    assert_eq!(
        lines(SRC, "E "),
        ["E f=1 g=1 x=1 s=1 cf=1 cx=1", "E miss f=0 x=0 s=0 cf=0"]
    );
}
