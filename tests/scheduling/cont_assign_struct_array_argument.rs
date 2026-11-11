//! IEEE 1800-2023 §10.3.2: a continuous assignment reads the member leaves
//! of a whole unpacked struct array passed by value (§7.2, §7.4.2/§7.4.4,
//! §13.4/§13.5.1). §10.9 supplies struct values; §4.7/§6.8 avoid time-zero
//! initialization and observation races. Expected sums follow these rules.

#[test]
fn whole_struct_array_arguments_follow_member_writes() {
    let sim = xezim::simulate(include_str!("cont_assign_struct_array_argument.sv"), 10_000)
        .expect("simulate failed");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    for expected in ["initial 7 10", "value 25", "tag 38", "unchanged 25 38"] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:#?}"
        );
    }
}

// Check values after settling (§4.7, §10.3.2), without counting evaluations.
fn check_probe(src: &str, expected_passes: usize) {
    let sim = xezim::simulate(src, 100_000).expect("simulate failed");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(
        !messages.iter().any(|m| m.starts_with("FAIL")),
        "failed checks: {messages:#?}"
    );
    assert_eq!(
        messages.iter().filter(|m| m.starts_with("PASS")).count(),
        expected_passes,
        "missing checks: {messages:#?}"
    );
}

#[test]
fn function_body_names_keep_their_dependencies() {
    check_probe(include_str!("cont_assign_function_body_names.sv"), 12);
}

#[test]
fn hierarchical_lhs_does_not_scope_call_actuals() {
    check_probe(include_str!("cont_assign_hierarchical_lhs_scope.sv"), 4);
}

#[test]
fn struct_array_port_arguments_keep_updating() {
    let sim = xezim::simulate(
        include_str!("cont_assign_struct_array_port_argument.sv"),
        100_000,
    )
    .expect("simulate failed");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    for expected in ["port initial 10 45", "port changed 15 41"] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:#?}"
        );
    }
}

// §7.2.2/§7.4.2: the selected whole struct remains an actual operand.
#[test]
fn indexed_struct_arguments_follow_selected_member_writes() {
    let sim = xezim::simulate(
        include_str!("cont_assign_indexed_struct_argument.sv"),
        100_000,
    )
    .expect("simulate failed");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    for expected in [
        "eval 1000",
        "eval 2000",
        "eval 3000",
        "eval 4000",
        "indexed 2",
    ] {
        assert!(
            messages.contains(&expected),
            "missing {expected:?}: {messages:#?}"
        );
    }
}

// §23.9, §27.5: resolve each actual in its own lexical scope.
#[test]
fn same_named_struct_array_actuals_use_local_scope() {
    check_probe(include_str!("cont_assign_struct_array_scopes.sv"), 5);
}

// §7.4.5, §10.9, §11.4.11: visit aggregate operands throughout actuals.
#[test]
fn struct_operands_in_actual_expressions_follow_member_writes() {
    check_probe(include_str!("cont_assign_struct_actual_expressions.sv"), 9);
}

// §10.3.2, §20.6/§20.7, §3.12.1: non-operands add no member wakes.
// Each guard passes on main; the isolated ignored-input observer returns $time.
#[test]
fn non_operand_struct_members_do_not_wake_calls() {
    check_probe(include_str!("cont_assign_struct_non_operand_scalar.sv"), 2);
    check_probe(include_str!("cont_assign_struct_non_operand_element.sv"), 2);
    check_probe(include_str!("cont_assign_struct_non_operand_bits.sv"), 2);
    check_probe(
        include_str!("cont_assign_struct_non_operand_typename.sv"),
        2,
    );
    check_probe(include_str!("cont_assign_struct_non_operand_size.sv"), 2);
    check_probe(
        include_str!("cont_assign_struct_non_operand_dimensions.sv"),
        2,
    );
    check_probe(
        include_str!("cont_assign_struct_non_operand_query_call.sv"),
        2,
    );
    check_probe(include_str!("cont_assign_struct_non_operand_unit.sv"), 2);
}

// §7.4.5/§10.3.2: variable-index scalar operands exclude sibling members.
// Passes on main; protects against unnecessary aggregate expansion.
#[test]
fn variable_scalar_actual_keeps_only_ordinary_reads() {
    check_probe(
        include_str!("cont_assign_struct_non_operand_variable_scalar.sv"),
        4,
    );
}

// §23.8/§23.9: block and instance spellings can be indistinguishable after
// elaboration. Assert the unrelated write only; retain the positive as a limit.
#[test]
fn ambiguous_block_actual_does_not_read_instance_members() {
    check_probe(include_str!("cont_assign_struct_ambiguous_block.sv"), 1);
}

// §23.3.1/§23.6/§10.3.2: rooted operands in children and at the top.
// Positive checks fail on main; controls run in separate designs.
#[test]
fn rooted_struct_actuals_use_the_absolute_declaration() {
    check_probe(include_str!("cont_assign_struct_child_root.sv"), 3);
    check_probe(include_str!("cont_assign_struct_child_topname.sv"), 3);
    check_probe(include_str!("cont_assign_struct_child_local.sv"), 3);
    check_probe(include_str!("cont_assign_struct_root_top.sv"), 3);
}

// §7.4.5/§10.3.2: retain an aggregate member path after a variable select.
#[test]
fn variable_struct_member_actual_excludes_siblings() {
    check_probe(include_str!("cont_assign_struct_variable_member.sv"), 3);
    check_probe(
        include_str!("cont_assign_struct_variable_member_element.sv"),
        3,
    );
}
