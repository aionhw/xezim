//! §10.9.1, §10.9.2, §26.3, §6.12: a bare type key of an assignment pattern
//! names the type visible at that point of the source. Covered: a wildcard
//! import that hides a compilation-unit type of the same name, a module
//! header import, a key written before and after a module-body import, an
//! import inside a generate loop, an explicit import, and `real` and
//! `realtime` keys as one type with the last one winning. The expected
//! values are those of the LRM; the reference simulator gives the same
//! lines except `gen outer bits`, where it lets the import of the generate
//! block reach the enclosing module (§26.3: an import is visible in the
//! scope it is declared in).

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn type_keys_follow_the_imports_in_scope() {
    let src = r#"
package p; typedef logic [3:0] PT; typedef int IT; localparam int K = 5; function automatic int f(int x); return x + K; endfunction endpackage
package q; typedef logic [7:0] PT; localparam int K = 9; endpackage
typedef logic [1:0] PT;
module other;
  import q::*;
  typedef struct { PT a; int b; } S;
  S s; int v[2];
  initial begin s = '{PT: 8'hAB, default: 7}; #1 $display("T| other %h %0d K=%0d", s.a, s.b, K); end
endmodule
module hdr import p::*; #(parameter int W = K) (input PT din);
  typedef struct { PT a; int b; } S;
  S s;
  initial begin s = '{PT: 4'h9, int: W}; #1 $display("T| hdr %h %0d %0d", s.a, s.b, $bits(din)); end
endmodule
module late;
  typedef struct { PT a; int b; } S0;   // $unit PT (2 bits) here
  S0 s0;
  import p::*;
  typedef struct { PT a; IT b; } S1;   // p::PT (4 bits)
  S1 s1;
  function automatic int g(); return f(1) + K; endfunction
  initial begin s0 = '{PT: 2'h2, default: 1}; s1 = '{PT: 4'hC, IT: 3}; #1 $display("T| late %0d %h %0d | %0d %h %0d g=%0d", $bits(s0.a), s0.a, s0.b, $bits(s1.a), s1.a, s1.b, g()); end
endmodule
module gen;
  for (genvar i = 0; i < 2; i++) begin : g
    import q::*;
    typedef struct { PT a; int b; } S;
    S s;
    initial begin s = '{PT: 8'h10 + i, default: K}; #1 $display("T| gen%0d %h %0d %0d", i, s.a, s.b, $bits(s.a)); end
  end
  PT outer;
  initial #1 $display("T| gen outer bits %0d", $bits(outer));
endmodule
module top;
  other o(); hdr h(4'h3); late l(); gen g();
  import p::PT;
  PT x;
  typedef struct { PT a; shortint b; real r; realtime rt; } S;
  S s;
  initial begin
    s = '{PT: 4'h5, shortint: 2, real: 1.5};
    #2 $display("T| top %0d %h %0d %g %g", $bits(x), s.a, s.b, s.r, s.rt);
    s = '{realtime: 2.5, default: 1};
    $display("T| top %h %0d %g %g", s.a, s.b, s.r, s.rt);
    s = '{real: 0.5, realtime: 3.5, default: 1};
    $display("T| top %h %0d %g %g", s.a, s.b, s.r, s.rt);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T| other ab 7 K=9",
            "T| hdr 9 5 4",
            "T| late 2 1 1 | 4 c 3 g=11",
            "T| gen0 10 9 8",
            "T| gen1 11 9 8",
            "T| gen outer bits 2",
            "T| top 4 5 2 1.5 1.5",
            "T| top 1 1 2.5 2.5",
            "T| top 1 1 3.5 3.5",
        ]
    );
}
