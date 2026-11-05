//! §27.4, §23.9: each iteration of a generate-for block has its own copy of
//! the functions and tasks declared in it, named `g[i].f`. A bare call to one
//! from inside the iteration (an `initial`, a continuous assign, another
//! subroutine, the function's own result variable or a nested block) names
//! that copy, as a call from outside through `g[i].f()` does. The bare call
//! was reported as an undeclared identifier. Expected lines were checked with
//! the reference simulator.
use xezim::simulate;

#[test]
fn calls_inside_a_generate_for_name_the_iterations_subroutines() {
    let result = simulate(include_str!("generate_block_subroutines.sv"), 100)
        .unwrap_or_else(|e| panic!("rejected: {e:?}"));
    let lines: Vec<&str> = result
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|blk 0 f=8 old=100 fact=6 rd=0",
            "T|task 0",
            "T|blk 1 f=9 old=101 fact=24 rd=10",
            "T|task 1",
            "T|nested 0 f=8",
            "T|nested 1 f=9",
            "T|cond cf=7",
            "T|assign 8 9",
            "T|outside 8 9 7",
        ]
    );
}
