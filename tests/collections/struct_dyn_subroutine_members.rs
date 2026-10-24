//! IEEE 1800-2017 §7.2, §7.5, §13.4/§13.5: the dynamic members of an
//! unpacked struct that lives in a subroutine — an automatic or static local,
//! a formal, an `output` formal — and of a struct handed between subroutines
//! by value.
//!
//! - A dynamic / queue member is a collection of its own (§7.5, §7.10), not a
//!   scalar member: an element write (`a.d[i] = v`) inside a function or task
//!   stores the element, which reads back in the subroutine and leaves with
//!   the struct through `return` (§13.4.1) or an `output` formal (§13.5).
//! - A function call that returns such a struct is a complete struct value
//!   as an actual (§13.5.1): the formal receives its dynamic members, also
//!   when the call is nested or other actuals name the caller's variables.

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
fn element_writes_in_functions_and_tasks() {
    assert_eq!(
        t_lines(include_str!("struct_dyn_subroutine_elems.sv")),
        [
            "T|fill-auto 3 65 66 67",
            "T|fill-static 2 97 98",
            "T|read-back 4243 -61",
            "T|task-local 22 5 3",
            "T|make-auto 3 3 22 1 5",
            "T|task-output 1000 1001",
            "T|make-static 2 2 1000 1001",
            "T|formal 165 caller 0 3",
        ]
    );
}

#[test]
fn returned_structs_as_actuals() {
    assert_eq!(
        t_lines(include_str!("struct_dyn_call_actuals.sv")),
        [
            "T|direct 'a' 'c'",
            "T|through 'h'",
            "T|nested 'de'",
            "T|mixed 'fa'",
            "T|swap 'b|a'",
            "T|assign 'ga' 2",
        ]
    );
}
