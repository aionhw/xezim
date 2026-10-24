//! IEEE 1800-2017 §7.8, §7.9.1–§7.9.6: `delete(index)`, `exists`,
//! `num`/`size` and `first`/`next` on associative arrays whose elements are
//! unpacked structs — with and without a dynamic member — and on arrays of
//! ints, with `longint` and `int` keys.
//!
//! `delete(index)` removes the entry (§7.9.2): it no longer `exists`
//! (§7.9.3), is no longer counted by `num()`/`size()` (§7.9.1) and is skipped
//! by `first`/`next` (§7.9.4, §7.9.5); the other entries are intact.

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
fn deleted_struct_entries_leave_the_array() {
    assert_eq!(
        t_lines(include_str!("assoc_dyn_struct_delete.sv")),
        [
            "T|heap 0 1 3 3",
            "T|heap2 0 1 2",
            "T|heap-first 3 1",
            "T|heap-next 5 122",
            "T|heap3 0 1",
            "T|int-key 0 1 -2 1",
            "T|plain-struct 0 1",
            "T|ints 0 1 0 1",
            "T|all 0 0",
        ]
    );
}
