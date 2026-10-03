//! Chained class-scope types (IEEE 1800-2017 §8.23, §8.25.1, §6.20.3,
//! §13.5): a class scope used as the prefix of a TYPE — `pkg::cls::td`,
//! `cls#(N)::td`, nested `pkg::Outer::Inner::td` — in every type
//! position: module/ANSI-port/function-port/task-body declarations,
//! function return types, class properties, typedef aliases, parameters,
//! queue elements, cast targets, struct members, type-parameter defaults
//! and enum variables. The parser used to collapse the chain to a single
//! `::` link and mis-route the statement to the expression parser, so
//! none of these parsed. The expression-position shapes (statics, enum
//! labels, methods) are guarded at the end of the same file. Expected
//! values come from the reference simulator.

use xezim::simulate;

#[test]
fn pkg_class_scope_types_full_matrix() {
    let sim = simulate(include_str!("pkg_class_scope_types.sv"), 1_000_000)
        .unwrap_or_else(|e| panic!("simulate failed: {}", e));
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    let fails: Vec<&str> = msgs
        .iter()
        .copied()
        .filter(|m| m.starts_with("FAIL"))
        .collect();
    assert!(
        fails.is_empty(),
        "pkg::class::TYPE chained-scope type regressions:\n{}",
        fails.join("\n")
    );
    assert!(
        msgs.iter().any(|m| *m == "TEST_PASS"),
        "never reached TEST_PASS — simulation died early:\n{}",
        msgs.join("\n")
    );
}
