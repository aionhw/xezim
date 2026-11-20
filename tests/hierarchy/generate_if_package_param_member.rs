//! IEEE 1800-2023 §27.5, §11.2.1, §26.3: a generate condition may read a
//! member of an element of a package parameter by its package-scoped name,
//! `p::Info[k].variant` with `k` a genvar. That is a constant expression (a
//! parameter, a constant select, a member select), whether or not the module
//! imports `p`. xezim rejected it with "Generate if condition must be a
//! constant expression" unless the module imported the package. The expected
//! branches follow the LRM, and Verilator 5.046 selects the same ones.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Exactly the expected branch messages, each once, in any order: a branch
/// that was not selected must not run.
fn expect_exactly(src: &str, want: &[&str]) {
    let mut got = lines(src);
    got.sort();
    let mut want: Vec<String> = want.iter().map(|w| w.to_string()).collect();
    want.sort();
    assert_eq!(got, want, "{src}");
}

const PKG: &str = "package p;
  typedef struct packed { logic [1:0] variant; logic [3:0] lanes; } info_t;
  parameter info_t Info [3] = '{'{2'd0, 4'd1}, '{2'd1, 4'd2}, '{2'd2, 4'd4}};
  localparam int unsigned N = 3;
endpackage
";

#[test]
fn generate_if_reads_a_scoped_struct_array_member() {
    let src = format!(
        "{PKG}module top;
  for (genvar k = 0; k < p::N; k++) begin : g
    if (p::Info[k].variant == 0) begin : b0
      initial $display(\"g[%0d] b0\", k);
    end else if (p::Info[k].variant == 1 && p::Info[k].lanes > 1) begin : b1
      initial $display(\"g[%0d] b1\", k);
    end else begin : b2
      initial $display(\"g[%0d] b2\", k);
    end
  end
endmodule
"
    );
    expect_exactly(&src, &["g[0] b0", "g[1] b1", "g[2] b2"]);
}

#[test]
fn generate_case_and_constant_index_read_the_scoped_member() {
    let src = format!(
        "{PKG}module top;
  for (genvar k = 0; k < 3; k++) begin : g
    case (p::Info[k].lanes)
      4'd1: begin : one initial $display(\"g[%0d] one\", k); end
      4'd2: begin : two initial $display(\"g[%0d] two\", k); end
      default: begin : other initial $display(\"g[%0d] other\", k); end
    endcase
  end
  if (p::Info[2].variant == 2) begin : last
    initial $display(\"last\");
  end
endmodule
"
    );
    expect_exactly(&src, &["g[0] one", "g[1] two", "g[2] other", "last"]);
}
