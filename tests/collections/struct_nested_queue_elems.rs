//! IEEE 1800-2017 §7.2, §10.9.2, §7.10.2.5/§7.10.2.6: queues whose elements
//! are unpacked structs with a nested unpacked-struct member.
//!
//! - An assignment-pattern item for a struct-typed member that is itself a
//!   struct of that type — a variable, a formal or a function result — is
//!   assigned to the member whole (§10.9.2); it is not spread over the
//!   nested members as a `default:` value would be.
//! - `pop_front()` / `pop_back()` returns the element, so a member of the
//!   result (`q.pop_front().v`, `q.pop_front().v.f0`) reads that element's
//!   member, a nested struct included, and the queue is popped once.

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
fn nested_struct_members_of_queue_elements() {
    assert_eq!(
        t_lines(include_str!("struct_nested_queue_elems.sv")),
        [
            "T|push 1 5 -1 113",
            "T|var 6 -1 113",
            "T|items -3 9 8 -1",
            "T|pop-member -1 113 3",
            "T|pop-leaf -3 8 1",
            "T|pop-back -1 113 0",
        ]
    );
}
