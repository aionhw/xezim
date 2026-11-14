//! IEEE 1800-2023 §5.7.1, §11.6.1, §11.8.1-2 and §10.9.1-2:
//! negative sized literals retain their type in array pattern index keys,
//! including nested patterns in a structure's unpacked array member.
//! Negative indices check the plain array and an unselected structure leaf;
//! reading a negative-index structure leaf is a separate existing limitation.
#[test]
fn signed_literal_pattern_indices() {
    let sim = xezim::simulate(include_str!("signed_literal_pattern_index.sv"), 100_000)
        .expect("simulate pattern indices");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("Simulation"))
        .collect();
    assert_eq!(
        lines,
        [
            "signed_and 7 7",
            "unsigned_and 7 7",
            "decimal 7 7",
            "unsized_hex 7 7",
            "unsigned_hex 7 7",
            "negative 0 7",
            "unary_negative 0 7",
            "positive 7 7",
            "octal 0 7",
            "hex 0 7",
            "based_decimal 0 7",
            "underscores 0 7",
            "add 7 7",
            "sub 0 7",
            "mul 0 7",
            "or 7 7",
            "xor 7 7",
            "not 7 7",
            "minus 7 7",
            "left 0 7",
            "right 7 7",
            "arith_right 0 7",
            "lt 7 7",
            "eq 7 7",
            "conditional 0 7",
            "mixed_unsigned 7 7",
            "wider 7 7",
            "narrower 0 7",
            "narrow_add 7 7",
            "conditional_unsigned 7 7",
            "binary_unsigned 7 7",
            "octal_unsigned 7 7",
            "decimal_unsigned 7 7",
            "wide_unsigned_mix 7 7",
            "nested_context 7 7",
            "comparison_context 7 7",
            "shift_wide 7 7",
        ]
    );
}
