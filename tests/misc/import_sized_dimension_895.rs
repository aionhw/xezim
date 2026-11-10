//! IEEE 1800-2023 §26.3, §7.4.2: a module's own imports are searched before
//! the compilation unit's wildcard import when a name sizes an unpacked array
//! `a[N]` — the explicit `import q::N;` (and a module's own `import q::*;`)
//! wins over the file-scope `import r::*;` for the dimension, as it already
//! did for the value and for packed dimensions (#895). Expected values are
//! the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn module_import_sizes_an_unpacked_array_over_the_unit_wildcard() {
    let o = out(r#"
package r; localparam int N = 3; localparam int K = 30; endpackage
package q; localparam int N = 5; localparam int K = 50; endpackage
import r::*;
module leaf;
  import q::N;
  int c [N];
  initial $display("T|leaf N=%0d size=%0d", N, $size(c));
endmodule
module child;
  import q::N;
  int a [N];
  logic [N-1:0] v;
  typedef logic [N-1:0] t;
  t tv;
  localparam int L = N * 2;
  logic [L-1:0] lv;
  int b [L];
  leaf u_leaf();
  genvar i;
  int cnt = 0;
  for (i = 0; i < N; i++) begin : g
    initial cnt++;
  end
  initial begin
    #1;
    $display("T|N=%0d size=%0d bitsv=%0d bitst=%0d L=%0d bitslv=%0d sizeb=%0d gen=%0d",
      N, $size(a), $bits(v), $bits(tv), L, $bits(lv), $size(b), cnt);
  end
endmodule
module wild;
  import q::*;
  int a [K];
  logic [K-1:0] v;
  initial $display("T|wild K=%0d size=%0d bits=%0d", K, $size(a), $bits(v));
endmodule
module top;
  child u();
  wild w();
endmodule
"#);
    assert_eq!(
        o,
        [
            "T|leaf N=5 size=5",
            "T|wild K=50 size=50 bits=50",
            "T|N=5 size=5 bitsv=5 bitst=5 L=10 bitslv=10 sizeb=10 gen=5",
        ]
    );
}

/// With `r::N = 0` the array still takes `q::N` (the issue's second shape).
#[test]
fn a_zero_unit_candidate_does_not_size_the_array() {
    let o = out(r#"
package r; localparam int N = 0; endpackage
package q; localparam int N = 5; endpackage
import r::*;
module child;
  import q::N;
  int a [N];
  initial $display("T|N=%0d size=%0d", N, $size(a));
endmodule
module top;
  child u();
endmodule
"#);
    assert_eq!(o, ["T|N=5 size=5"]);
}
