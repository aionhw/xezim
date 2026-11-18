//! §8.8, §8.25: `new` assigned to a class member through an object handle
//! or `this` constructs the specialization the member is declared with
//! (`mem_c #(8) m8; ... this.m8 = new;`, `u.m4 = new();`), as a bare
//! `m = new` in a method of the owner does, including a member inherited
//! from a base class and a member of a parameterized owner. Expected values
//! are the reference simulator's.

#[test]
fn member_new_builds_declared_specialization() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let sim = xezim::simulate(include_str!("member_new_specialization.sv"), 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|m16=16 m8=8 m4=4 inh=6 dflt=32 own=5 nxt=6"]);
}
