//! §8.24, §23.9 (#843): the body of an out-of-block method is in the scope
//! of its class, so a member it names by its bare name is the member, even
//! when the top module declares a variable of that name. The
//! use-before-declaration scan took the module's later declaration for the
//! name and stopped elaboration with "referenced before its declaration".

use xezim::simulate;

#[test]
fn out_of_block_method_members_are_not_module_variables() {
    let src = include_str!("unit_class_out_of_block_members_843.sv");
    let sim = simulate(src, 100).expect("simulate failed");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(!lines.iter().any(|l| l.starts_with("FAIL")), "{lines:#?}");
    assert_eq!(lines.last().copied(), Some("ALL PASS"), "{lines:#?}");
}
