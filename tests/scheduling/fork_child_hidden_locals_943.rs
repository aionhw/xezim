//! §6.21, §9.3.2 (#943): a local that a block declares hides an enclosing
//! variable of the same name for the block's extent, and a fork child's block
//! declaration hides its parent's variable from that child only. A fork child
//! runs on a copy of the frames it shares with its parent, and its writes were
//! merged back by name, so a child's `payload` overwrote the parent's
//! same-named local (a struct, string or class handle with it); the name-keyed
//! type metadata of the two declarations was restored by frame generation,
//! which parent and child share, so each saw the other's type half the time.
//! Lines are compared sorted: the order of same-time displays of different
//! processes is not fixed (§4.7). Expected values are the reference
//! simulator's.

use xezim::simulate;

fn sorted_t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

/// The cousin kinds of #943 with their values: a parent task and its
/// `join_none` child both declare `payload` (unpacked struct, packed struct,
/// string, class handle, int, and int/string), and both keep running.
#[test]
fn fork_child_block_local_hides_parent_local_of_same_name() {
    let got = sorted_t_lines(include_str!(
        "../issue_cases/fork_child_hidden_locals_943_cousins.sv"
    ));
    let want = [
        "T|k1.1 child a=0 b=102",
        "T|k1.1 child a=0 b=103",
        "T|k1.1 parent a=11 b=0",
        "T|k1.1 parent a=21 b=0",
        "T|k1.2 child a=0 b=103",
        "T|k1.2 child a=0 b=104",
        "T|k1.2 parent a=12 b=0",
        "T|k1.2 parent a=22 b=0",
        "T|k2.1 child xx42xxxx",
        "T|k2.1 child xx43xxxx",
        "T|k2.1 parent 11xxxxxx",
        "T|k2.1 parent 21xxxxxx",
        "T|k3.1 child child+ 6",
        "T|k3.1 child child++ 7",
        "T|k3.1 parent parent- 7",
        "T|k3.1 parent parent-- 8",
        "T|k4.1 child f=52",
        "T|k4.1 child f=53",
        "T|k4.1 parent f=4",
        "T|k4.1 parent f=7",
        "T|k8.1 child 1002",
        "T|k8.1 child 1003",
        "T|k8.1 parent 15",
        "T|k8.1 parent 8",
        "T|k8.2 child 1003",
        "T|k8.2 child 1004",
        "T|k8.2 parent 16",
        "T|k8.2 parent 9",
        "T|kx.1 child st",
        "T|kx.1 child stt",
        "T|kx.1 parent 15 32",
        "T|kx.1 parent 8 32",
    ];
    assert_eq!(got, want);
}

/// Same-named locals of different types in two live frames: parent and fork
/// child, two nested task frames of one process, and two processes.
#[test]
fn same_named_locals_of_different_types_keep_their_own_types() {
    let got = sorted_t_lines(include_str!(
        "../issue_cases/fork_child_hidden_locals_943_types.sv"
    ));
    let want = [
        "T|A0 child payload=childx len=6",
        "T|A0 child payload=childxx len=7",
        "T|A0 child payload=childxxx len=8",
        "T|A0 parent payload=41 bits=32",
        "T|A0 parent payload=42 bits=32",
        "T|B child x=12 y=35 whole=1235 bits=16",
        "T|B child x=12 y=36 whole=1236 bits=16",
        "T|B child x=12 y=37 whole=1237 bits=16",
        "T|B parent a=7 b=10",
        "T|B parent a=7 b=11",
        "T|C child a=101 b=200",
        "T|C child a=102 b=200",
        "T|C child a=103 b=200",
        "T|C parent f=6",
        "T|C parent f=7",
        "T|D inner x=a2 y=b2 bits=16",
        "T|D inner x=a3 y=b2 bits=16",
        "T|D outer a=1 b=12",
        "T|E geom x=01 y=04",
        "T|E geom x=01 y=06",
        "T|E geom x=01 y=08",
        "T|E str payload=st",
        "T|E str payload=stt",
        "T|E str payload=sttt",
    ];
    assert_eq!(got, want);
}

/// Three views of one frame, a grandchild hiding its parent's hiding local,
/// block shadowing in one frame while another process runs a same-named
/// local, and a sibling's write of a variable that the other sibling's block
/// hides, which that sibling sees once its block ends.
#[test]
fn hidden_and_shared_variables_in_fork_children() {
    let got = sorted_t_lines(include_str!(
        "../issue_cases/fork_child_hidden_locals_943_neighbours.sv"
    ));
    let want = [
        "T|Q c1 0a0c",
        "T|Q c1 0a0d",
        "T|Q c2 a=12 b=4",
        "T|Q c2 a=6 b=4",
        "T|Q c3 shared=2",
        "T|Q c3 shared=3",
        "T|Q parent payload=3",
        "T|R child mid.",
        "T|R child mid..",
        "T|R grandchild 71",
        "T|R grandchild 72",
        "T|R parent 4",
        "T|R parent 5",
        "T|R parent 6",
        "T|S inner in+ 3",
        "T|S other 1222",
        "T|S other 1322",
        "T|S other 1422",
        "T|S outer 15 32",
        "T|U parent payload=103",
        "T|U s1 inner h",
        "T|U s1 outer 3",
        "T|U s2 shared=2",
        "T|U s2 shared=3",
    ];
    assert_eq!(got, want);
}
