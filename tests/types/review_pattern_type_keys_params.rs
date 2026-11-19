//! IEEE 1800-2023 §10.9.1, §10.9.2: type keys in the pattern of an unpacked
//! array parameter. A type key that matches no sub-array descends into each
//! row an index key does not set, a `default:` value that reaches a
//! structure element with type keys is the default of its members, and a
//! type key of an outer level takes precedence over the items of a
//! `default:` row pattern, as it does for a variable.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("Simulation"))
        .collect()
}

#[test]
fn parameter_rows_take_outer_type_keys() {
    let src = r#"
module top;
  typedef struct {int x; byte y;} s_t;
  localparam int P5 [2][3] = '{int: 8, 1: '{default: 2}};
  localparam s_t P10 [2] = '{int: 5, default: 0};
  localparam int P17 [2][2] = '{int: 2, default: '{3, 4}};
  int v17 [2][2];
  initial begin
    v17 = '{int: 2, default: '{3, 4}};
    $display("P5=%p", P5);
    $display("P10=%0d,%0d|%0d,%0d", P10[0].x, P10[0].y, P10[1].x, P10[1].y);
    $display("P17=%p v17=%p", P17, v17);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "P5='{'{8, 8, 8}, '{2, 2, 2}}",
            "P10=5,0|5,0",
            // The reference simulator gives both `'{'{3, 4}, '{3, 4}}`.
            "P17='{'{2, 2}, '{2, 2}} v17='{'{2, 2}, '{2, 2}}",
        ]
    );
}
