//! #309, #310, #311 (reported by Ganesh-TNL): unpacked structs whose members
//! are queues, dynamic or associative arrays, and package-scope aggregate
//! initializers.
//!
//! - #309 (§6.8, §7.9.11, §10.9.2): a package-scope initializer of an
//!   unpacked struct or an associative array was dropped; it now runs once
//!   before any initial block, like a module-scope one.
//! - #310 (§10.9.2, §7.2.1): an assignment pattern skipped a struct's
//!   dynamic members; each is now assigned as a whole collection, replacing
//!   the old contents.
//! - #311 (§7.10, §7.8, §11.4.12): a dynamic member of a struct ELEMENT of an
//!   associative array (`aa[k].q`) was not a collection; such members are now
//!   classified from the declared types wherever the struct lives.
//!
//! Expected values come from the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn t_lines(src: &str) -> Vec<String> {
    lines(src)
        .into_iter()
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn verdict(src: &str) -> Vec<String> {
    lines(src)
        .into_iter()
        .filter(|m| m.starts_with("TEST_") || m.starts_with("[ERROR]"))
        .collect()
}

#[test]
fn issue_309_package_struct_and_assoc_initializers() {
    assert_eq!(
        verdict(include_str!("struct_dyn_issue_309.sv")),
        ["TEST_PASS"]
    );
}

#[test]
fn issue_310_struct_pattern_assigns_queue_members() {
    assert_eq!(
        verdict(include_str!("struct_dyn_issue_310.sv")),
        ["TEST_PASS"]
    );
}

#[test]
fn issue_311_queue_members_of_assoc_struct_elements() {
    assert_eq!(
        verdict(include_str!("struct_dyn_issue_311.sv")),
        ["TEST_PASS"]
    );
}

/// Queue, dynamic and associative members; nested structs; initializers,
/// patterns, `default:` and copies (§7.2.1, §10.9.2).
#[test]
fn members_initializers_patterns_and_copies() {
    let expected = [
        "T|q1init x=3 sz=3 e2=6",
        "T|d1init x=1 sz=2 e1=8",
        "T|a1init x=2 n=1 k=9",
        "T|n1init y=1 x=2 sz=2 e1=4",
        "T|d1pat x=4 sz=3 e2=3",
        "T|a1pat x=5 n=2 ex_z=0 m=3",
        "T|n1pat y=7 x=8 sz=1 e0=1",
        "T|n2push sz=2 e1=6",
        "T|n2inpat x=1 sz=3 e2=4",
        "T|n2clr sz=0",
        "T|qddef x=0 sz=0",
        "T|m1def x=2 qsz=0 dsz=1",
        "T|q1fromq sz=3 e2=7 qdsz=4",
        "T|n2copy sz=2 n1sz=1",
        "T|ddcopy sz=3 e0=77 d1e0=1",
        "T|adcopy n=3 a1n=2",
        "T|n1sum 1",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_members.sv")), expected);
}

/// Structs with a queue member held in queues, dynamic, fixed and associative
/// arrays, and in class properties (§7.10, §7.8, §11.4.12).
#[test]
fn structs_in_containers_and_class_properties() {
    let expected = [
        "T|q0 sz=2 q1 sz=1 e=3",
        "T|q0push sz=3 e2=9",
        "T|q1asg sz=3 e2=6",
        "T|q1pat x=3 sz=1",
        "T|da1 sz=2 e1=5 da0=0",
        "T|da0pat sz=3",
        "T|fa2 sz=2 fa0 sz=1 fa1 sz=0",
        "T|fa1pat sz=4 e3=4",
        "T|aa5 sz=2 e1=11 aa6 sz=1 e0=12 n=2",
        "T|aasa sz=4 e0=0 e3=3",
        "T|aasa2 sz=2 e0=1",
        "T|concat sz=7 sum=36",
        "T|aascopy b=3 a=2",
        "T|cp x=1 sz=2",
        "T|cp2 sz=3 pa1=1 paa=2 e1=4",
        "T|cppat x=9 sz=0",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_containers.sv")), expected);
}

/// Struct, associative and struct-with-queue initializers in package, module,
/// class, automatic and static scopes (§6.8, §6.21).
#[test]
fn initializers_in_every_scope() {
    let expected = [
        "T|pkg ps=9,11 pd=4,4 pa=2,11,22 pai=1,5,3",
        "T|pkg pas=1,1,2 pq=1,2,3",
        "T|pkg pfa=2,3 pqq=1,6 getx=9",
        "T|mod ms=1,2 ma=1,1 mq=3,3,6",
        "T|cls cs=7,8 ca=2,4 cq=1,9",
        "T|auto ls=21,22 la=1 lq=2",
        "T|auto ls=21,22 la=1 lq=2",
        "T|static1 ss=31,32 sa=1 sq=1",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_scopes.sv")), expected);
}

/// `%p`, `==`, input/output/ref formals and struct return values (§21.2.1.7,
/// §11.4.5, §13.5, §13.4.1).
#[test]
fn print_compare_and_subroutine_formals() {
    let expected = [
        "T|pa '{x:1, ql:'{2, 3}}",
        "T|eq1 1",
        "T|eq2 0",
        "T|sum 6 asz=2",
        "T|mk x=3 sz=3 e2=3",
        "T|grow sz=4 e3=42",
        "T|ret x=5 sz=2 e1=6",
        "T|pc '{x:5, ql:'{5, 6}}",
        "T|paa 2 4 '{\"p\":'{x:1, y:2}, \"q\":'{x:3, y:4}}",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_ops.sv")), expected);
}

/// Direct operations on dynamic members of struct variables, nested structs and
/// fixed array elements (§7.2.1).
#[test]
fn member_operations_on_variables() {
    let expected = [
        "T|q1 sz=2 e1=4",
        "T|d1 sz=3 e2=5",
        "T|a1 n=2 k=9 ex=1",
        "T|n1 sz=2 e1=2",
        "T|fa sz=1 e0=7",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_basic.sv")), expected);
}

/// Associative members: traversal, delete, patterns and copies, through a
/// variable and an associative element (§7.9).
#[test]
fn associative_members() {
    let expected = [
        "T|first j",
        "T|next k",
        "T|last z",
        "T|del n=2 ex=0",
        "T|e n=2 m=2 q=2 ex=1",
        "T|e sum=3",
        "T|e asg n=1 r=5",
        "T|e pat n=3 e7=2 b=20",
        "T|copy n=2 a=10",
        "T|p '{x:1, m:'{\"a\":10, \"b\":20}}",
    ];
    assert_eq!(
        t_lines(include_str!("struct_dyn_assoc_members.sv")),
        expected
    );
}

/// Assignment patterns to unpacked-struct class properties, nested and with
/// queue members (§10.9.2, §8.4).
#[test]
fn class_struct_property_patterns() {
    let expected = [
        "T|a 1 2",
        "T|p 3 2 5",
        "T|n 1 2 1",
        "T|p2 7 3",
        "T|p3 9 1 1",
        "T|a2 5 6",
        "T|r 0 0 7 7",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_class_props.sv")), expected);
}

/// Queue methods that move struct elements, and whole copies of queues and
/// associative arrays of structs, carry the queue members by value (§7.2.1,
/// §7.10.2, §7.9.9).
#[test]
fn moves_and_whole_copies_of_struct_elements() {
    let expected = [
        "T|pf 0 1 2",
        "T|ins 0 3 1 2",
        "T|del 3 1 2 n=3",
        "T|pop x=9 sz=3 n=2",
        "T|qcopy 2 1",
        "T|acopy 3 2",
        "T|elem 2 2",
        "T|elemw 2 2",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_moves.sv")), expected);
}

/// Struct formals with a queue member: handed on by `ref`, copied for an
/// `input` (also recursively), and returned through an `output` (§13.5).
#[test]
fn formals_handed_on_and_recursive() {
    let expected = [
        "T|outer sz=2 e1=1",
        "T|peek 3 gsz=2",
        "T|depth 1205 gsz=2",
        "T|fill x=3 sz=3 e2=3",
    ];
    assert_eq!(t_lines(include_str!("struct_dyn_formals.sv")), expected);
}
