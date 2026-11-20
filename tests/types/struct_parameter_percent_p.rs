//! IEEE 1800-2023 §21.2.1.7, §6.20.2: `%p` of a value parameter declared
//! with a structure type prints it member by member; the parameter holds the
//! members packed, and printed as one number.

#[test]
fn structure_parameters_print_member_by_member() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct {int x; byte y;} s_t;
  typedef struct packed {int a; byte b;} pi_t;
  localparam s_t P9 = '{int: 3, byte: 4};
  localparam s_t P9b = '{3, -4};
  parameter pi_t PI = '{a: -2, b: 5};
  initial $display("T|%p %p %p", P9, P9b, PI);
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|'{x:3, y:4} '{x:3, y:-4} '{a:-2, b:5}"]);
}
