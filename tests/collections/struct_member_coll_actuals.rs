//! IEEE 1800-2017 §7.2.1, §7.5, §7.10, §13.5: a dynamic-array or queue
//! member of an unpacked struct passed as the actual of a dynamic-array or
//! queue formal.
//!
//! The member is a collection of its own (§7.5, §7.10), named by its path
//! wherever the struct lives — a module variable, a nested struct, or a
//! subroutine's struct formal. An `input` formal receives a copy of its
//! elements (§13.5.1), a `ref` formal aliases it (§13.5.2) and an `output`
//! formal is copied back to it on return (§13.5).

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
fn struct_members_bind_collection_formals() {
    assert_eq!(
        t_lines(include_str!("struct_member_coll_actuals.sv")),
        [
            "T|conv 3 20 22 2",
            "T|input 63 63",
            "T|ref 120 3",
            "T|output 2 0 55",
            "T|queue 3 1203",
            "T|nested 2 902",
        ]
    );
}
