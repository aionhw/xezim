//! IEEE 1800-2023 §11.4.11, §7.2, §13.4.1: a conditional expression whose
//! operands are unpacked structs with a dynamic member.
//!
//! With a known condition the result is the selected operand (§11.4.11), a
//! struct whose dynamic member is part of its value (§7.2, §7.5): passed as
//! an actual, assigned (also into an array element and a queue), returned
//! (§13.4.1) and through nested conditionals, from call or variable
//! operands. A function returning a module-scope struct variable returns
//! all of its members.

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
fn conditional_struct_operands_keep_dynamic_members() {
    assert_eq!(
        t_lines(include_str!("struct_cond_operands.sv")),
        [
            "T|arg 3:abc 2:de",
            "T|assign 6:assign",
            "T|return 4:zero 5:other",
            "T|nested 1:a 2:bb 3:ccc",
            "T|var 6:double 3:vee",
            "T|return-var 5:local 6:double",
            "T|return-module 6:double",
            "T|nested-assign 2:qq",
            "T|elem 4:elem 3:vee",
        ]
    );
}
