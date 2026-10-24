//! IEEE 1800-2017 §7.2, §7.5, §7.10, §10.9, §13.4/§13.5: unpacked structs
//! whose dynamic members are dynamic arrays (or queues) of structs that have
//! dynamic members of their own — `struct { ...; A d[]; }` with
//! `A = struct { ...; byte unsigned d[]; }`, the shape VHDL arrays of
//! vectors and multi-dimensional arrays lower to.
//!
//! - A struct assignment copies every member (§7.2), and a dynamic-array
//!   member copies its elements by value (§7.5); an element that is itself a
//!   struct copies all of its members, its own dynamic members included.
//!   This holds for `h = g`, a struct handed to or returned from a function
//!   or task (§13.4.1, §13.5), and the same shapes one level deeper.
//! - A function result assigned to an element of a dynamic array of structs
//!   (`arr[0] = mk(...)`, §7.5) or inserted into a queue (`push_back`,
//!   `push_front`, `insert`, §7.10.2) keeps its dynamic members.

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
fn nested_dynamic_struct_arrays_copy_whole() {
    assert_eq!(
        t_lines(include_str!("struct_nested_dyn_copies.sv")),
        [
            "T|rows 2 3 42",
            "T|cp 5 2 2 42",
            "T|copy 5 2 2 41",
            "T|deep 3 2 2 42",
            "T|deep-asg 2 3 40",
            "T|dyn-of-AA 6 2 3",
            "T|dyn-of-AA-var 1 42",
            "T|queue-A 7 3 41 1",
            "T|queue-AA 8 2 3 1",
            "T|queue-AA-elem 1 42",
        ]
    );
}

#[test]
fn nested_dynamic_struct_arrays_through_inserts_and_tasks() {
    assert_eq!(
        t_lines(include_str!("struct_nested_dyn_inserts.sv")),
        [
            "T|front 3 2 3 1",
            "T|front-d 2 2 8",
            "T|task-out 6 2 4 9",
            "T|task-copy 6 4 2",
        ]
    );
}
