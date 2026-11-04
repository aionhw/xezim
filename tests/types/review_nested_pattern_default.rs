//! IEEE 1800-2017 §10.9.2: a packed-struct assignment pattern with a
//! nested struct member, compiled (`always_comb`) and interpreted
//! (`initial`). `default:` fills each member of a nested struct member;
//! the whole member took the value, so only its LSB was set.
//! Review follow-up to #589 / PR #590.
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("C ") || m.starts_with("I "))
        .collect()
}

/// Named nested patterns, `default:` over a nested member, an ordered
/// pattern, and a whole nested member, on both paths.
#[test]
fn nested_member_patterns_on_both_paths() {
    let src = r#"typedef struct packed { logic [7:0] cause; logic valid; } exc_t;
typedef struct packed { logic [1:0] a; exc_t e; } deep_t;
typedef struct packed {
  logic       req_ready;
  logic       resp_valid;
  logic [7:0] result;
  logic [3:0] trans_id;
  exc_t       exception;
  logic       fflags_valid;
} nest_t;
typedef struct packed { logic [3:0] x; deep_t d; logic y; } two_t;
module tb;
  logic [3:0] id;
  nest_t c1, c2, c3, c4, i1, i2, i3, i4;
  two_t c5, i5;
  always_comb c1 = '{trans_id: id, exception: '{cause: 8'h5a, valid: 1'b1}, default: '0};
  always_comb c2 = '{trans_id: id, default: 1'b1};
  always_comb c3 = '{1'b1, 1'b0, 8'h33, id, '{8'h44, 1'b1}, 1'b1};
  always_comb c4 = '{exception: 9'h1ff, default: '0};
  always_comb c5 = '{x: id, d: '{a: 2'b10, e: '{cause: 8'h77, valid: 1'b0}}, y: 1'b1};
  initial begin
    id = 4'h9;
    #1;
    i1 = '{trans_id: id, exception: '{cause: 8'h5a, valid: 1'b1}, default: '0};
    i2 = '{trans_id: id, default: 1'b1};
    i3 = '{1'b1, 1'b0, 8'h33, id, '{8'h44, 1'b1}, 1'b1};
    i4 = '{exception: 9'h1ff, default: '0};
    i5 = '{x: id, d: '{a: 2'b10, e: '{cause: 8'h77, valid: 1'b0}}, y: 1'b1};
    $display("C %h %h %h %h %h", c1, c2, c3, c4, c5);
    $display("I %h %h %h %h %h", i1, i2, i3, i4, i5);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        shown(src),
        [
            "C 00256a c06407 8ce513 0003fe 99dd",
            "I 00256a c06407 8ce513 0003fe 99dd"
        ]
    );
}

/// `default:` two levels down, wider than the leaves, and negative.
#[test]
fn default_fills_every_nested_level() {
    let src = r#"typedef struct packed { logic [7:0] cause; logic valid; } exc_t;
typedef struct packed { logic [1:0] a; exc_t e; } deep_t;
typedef struct packed { logic [3:0] x; deep_t d; logic y; } two_t;
module tb;
  logic [3:0] id;
  two_t c1, c2, c3, i1, i2, i3;
  always_comb c1 = '{x: id, default: 2'b11};
  always_comb c2 = '{y: 1'b0, default: 8'hab};
  always_comb c3 = '{d: '{a: 2'b01, default: 1}, default: -1};
  initial begin
    id = 4'h5; #1;
    i1 = '{x: id, default: 2'b11};
    i2 = '{y: 1'b0, default: 8'hab};
    i3 = '{d: '{a: 2'b01, default: 1}, default: -1};
    $display("C %h %h %h", c1, c2, c3);
    $display("I %h %h %h", i1, i2, i3);
    $finish;
  end
endmodule
"#;
    assert_eq!(shown(src), ["C 5c0f beae f407", "I 5c0f beae f407"]);
}
