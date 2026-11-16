//! IEEE 1800-2023 §10.10: struct concatenation elements update live cells.
//! §10.9.1 / §10.9.2 / §11.4.11 cover the neighbouring pattern and conditional paths;
//! §9.4.2 requires member value-change event controls to observe the writes.

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
fn fixed_struct_concat_updates_registered_members() {
    check(
        include_str!("struct_concat_registered/fixed.sv"),
        &[
            "T|control|3 bee",
            "T|ids|3 7 -9",
            "T|labels|bee cee a much longer third string",
            "T|reals|1.25 2.50 -3.75",
            "T|packed|35 79 db",
            "T|packed_members|3 5 7 9 13 11",
            "T|data|12 22 32",
            "T|alias|-9 3 7",
            "T|descending|3 7 -9",
        ],
    );
}

#[test]
fn struct_concat_updates_module_collections_and_keeps_map_backed_locals() {
    check(
        include_str!("struct_concat_registered/collections.sv"),
        &[
            "T|local|2 3 cee 2 3 cee",
            "T|reorder|7 bee 7 bee",
            "T|module|2 3 cee 2 3 cee",
            "T|module_reorder|7 bee 7 bee",
            "T|guards|3 7 1 2 3 4",
        ],
    );
}

#[test]
fn struct_array_patterns_and_conditionals_materialize_members() {
    check(
        include_str!("struct_concat_registered/materialization.sv"),
        &[
            "T|pattern|3 bee 7 cee",
            "T|typed|7 cee 3 bee",
            "T|true|3 bee 7 cee",
            "T|false|7 cee 3 bee",
            "T|conditional_pattern|7 cee 3 bee",
            "T|typed_formal|37",
            "T|ambiguous|0 0",
        ],
    );
}

#[test]
fn struct_concat_in_functions_reaches_compiled_reads() {
    check(
        include_str!("struct_concat_registered/execution.sv"),
        &[
            "T|function|37",
            "T|read|37",
            "T|comb_read|37",
            "T|bytecode|37",
        ],
    );
}

#[test]
fn struct_concat_wakes_member_value_change_wait() {
    check(
        include_str!("struct_concat_registered/events.sv"),
        &["T|wake|3 7", "T|events|1"],
    );
}
