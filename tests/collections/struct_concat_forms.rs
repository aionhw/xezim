//! IEEE 1800-2023 §10.10: each assignment-compatible struct operand is one element.
//! §§7.5.1, 7.8.6, 7.8.7 and 7.10 cover collection allocation and missing keys;
//! §§6.20, 6.21, 7.2.1, 10.9.2, 11.7, 13.5.2 and 26 cover the operand and target forms.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn struct_concat_replaces_preallocated_dynamic_elements() {
    check(
        include_str!("struct_concat_registered/module_new_queue.sv"),
        &["T|module|3 7", "T|new|2 3 7", "T|queue|2 3 7"],
    );
}

#[test]
fn struct_concat_in_automatic_locals_and_ref_formals() {
    check(
        include_str!("struct_concat_registered/automatic_ref.sv"),
        &["T|task|3 7", "T|function|37", "T|ref|3 7"],
    );
}

#[test]
fn struct_concat_preserves_associative_members_and_missing_keys() {
    check(
        include_str!("struct_concat_registered/nested_associative.sv"),
        &["T|assoc_members|3 7 31 72", "T|missing|0 0"],
    );
}

#[test]
fn struct_concat_reads_associative_operands_without_allocating_missing_keys() {
    check(
        include_str!("struct_concat_registered/associative_operands.sv"),
        &["T|assoc_operand|3 7 0 0"],
    );
}

#[test]
fn struct_concat_keeps_package_targets() {
    check(
        include_str!("struct_concat_registered/package_target.sv"),
        &["T|package|3 7"],
    );
}

#[test]
fn struct_concat_keeps_packed_constants_and_member_signedness() {
    check(
        include_str!("struct_concat_registered/packed_constants.sv"),
        &["T|packed_constants|-9 3 7 13 -9 -9"],
    );
}

#[test]
fn struct_concat_materializes_unpacked_literals_and_constants() {
    check(
        include_str!("struct_concat_registered/unpacked_constants.sv"),
        &["T|unpacked_constants|-9 3 7 13"],
    );
}

#[test]
fn struct_concat_materializes_signed_and_unsigned_member_patterns() {
    check(
        include_str!("struct_concat_registered/unpacked_signed_members.sv"),
        &["T|signed_members|-9 -9"],
    );
}
