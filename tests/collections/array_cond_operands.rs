//! IEEE 1800-2023 §11.4.11, §7.6: a conditional expression whose operands
//! are whole unpacked arrays. With a known condition the result is the
//! selected operand, assigned element by element (it was evaluated as one
//! value, which an unpacked array does not have, and assigned zeros), also
//! when the conditional is parenthesized.

use xezim::simulate;

fn t_lines(source: &str) -> Vec<String> {
    simulate(source, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|line| line.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn conditional_array_operands_assign_the_selected_array() {
    assert_eq!(
        t_lines(include_str!("array_cond_operands.sv")),
        [
            "T|then 1 2 3",
            "T|else 4 5 6",
            "T|nested 4 5 6",
            "T|paren 4 5 6",
            "T|2d 5 6 7 8",
        ]
    );
}
