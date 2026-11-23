//! §7.2.1: nested two-state packed members read X and Z as zero.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn nested_leaf_converts_in_direct_and_continuous_reads() {
    check(
        r#"
module t;
  typedef struct packed { bit b; logic [2:0] f; } inner_t;
  typedef struct packed { logic tag; inner_t inner; bit top_b; } s_t;
  s_t s;
  wire nested_w = s.inner.b;
  bit nested_c;
  always_comb nested_c = s.inner.b;
  initial begin
    s = 'x;
    #1;
    $display("display: inner.b=%b top_b=%b", s.inner.b, s.top_b);
    $display("assign:  inner.b=%b", nested_w);
    $display("comb:    inner.b=%b", nested_c);
  end
endmodule
"#,
        &[
            "display: inner.b=0 top_b=0",
            "assign:  inner.b=0",
            "comb:    inner.b=0",
        ],
    );
}

#[test]
fn nested_signed_conversion_preserves_other_fields() {
    check(
        r#"
module state_neighbor;
  typedef struct packed { bit signed [1:0] v; logic [1:0] keep; } inner_t;
  typedef struct packed { logic pad; inner_t nested; } outer_t;
  outer_t s;
  wire signed [1:0] w = s.nested.v;
  function automatic logic signed [1:0] read_v();
    return s.nested.v;
  endfunction
  initial begin
    s = 5'b1z1xz;
    #1 $display("STATE direct=%0d call=%0d wire=%0d four=%b",s.nested.v,read_v(),w,s.nested.keep);
    s = 5'b11110;
    #1 $display("STATE signed=%0d call=%0d wire=%0d four=%b",s.nested.v,read_v(),w,s.nested.keep);
  end
endmodule
"#,
        &[
            "STATE direct=1 call=1 wire=1 four=xz",
            "STATE signed=-1 call=-1 wire=-1 four=10",
        ],
    );
}
