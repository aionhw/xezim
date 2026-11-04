//! #321, #322, #323 (reported by Ganesh-TNL) and their siblings: typedef'd
//! collection types, fixed arrays of collections and static collections of
//! structs in parameterized classes.
//!
//! - #321 (§8.9, §8.25, §7.10): `C#(int)::q[0].m` read the members of a
//!   static struct queue of a parameterized class from no store; it now names
//!   the specialization's store, as the class's own static methods do.
//! - #322 (§7.4.5, §7.12.1): an element of a fixed array of queues
//!   (`int q[3][$]`, a struct member `T sp[8][$]`) is a collection of its own:
//!   whole assignment, locator results, methods, `foreach` and `inside`.
//! - #323 (§6.18, §7.2.1, §7.10): a struct member declared with a queue /
//!   dynamic / associative typedef, and a package variable of such a typedef,
//!   are collections like the directly declared forms.
//!
//! Expected values come from the reference simulator; `#[ignore]` pins are
//! confirmed gaps with the reference's output.
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
        .filter(|m| m.starts_with("TEST") || m.starts_with("FAIL") || m.starts_with("[ERROR]"))
        .collect()
}

#[test]
fn issue_321_static_struct_queue_of_parameterized_class() {
    assert_eq!(
        verdict(include_str!("spec_static_issue_321.sv")),
        ["TEST PASSED"]
    );
}

#[test]
fn issue_322_queue_members_through_fixed_array_elements() {
    assert_eq!(
        verdict(include_str!("fixed_coll_issue_322.sv")),
        ["TEST PASSED"]
    );
}

#[test]
fn issue_323_typedef_queue_in_assoc_of_structs() {
    assert_eq!(
        verdict(include_str!("typedef_coll_issue_323.sv")),
        ["TEST PASSED"]
    );
}

#[test]
fn spec_static_struct_collections() {
    assert_eq!(
        t_lines(include_str!("spec_static_structs.sv")),
        [
            "T|01 size=2 s0=a v0=7 s1=b v1=9",
            "T|03 sum=16",
            "T|04 aa num=1 s=x v=3",
            "T|05 da size=1 v=11",
            "T|08 nq size=1 in.s=n in.v=14 k=4",
            "T|09 mq size=1 v=15 s=m maa=15",
            "T|10 Q3 size=1 a=5 b=15",
            "T|11 Q4 size=1 a=6 b=24 Q3 size=1",
            "T|12 Qdef size=1 b=2",
            "T|14 Pbyte v=100 s=y",
            "T|15 pop s=a v=7 size=1",
            "T|16 insert s0=a s1=b",
            "T|17 find size=1 s=b",
            "T|18 write v=99",
            "T|19 deleted size=0",
        ]
    );
}

#[test]
fn fixed_array_of_queues_struct_members() {
    assert_eq!(
        t_lines(include_str!("fixed_coll_struct_members.sv")),
        [
            "T|01 sizes fa=4 3 s=4 3 m=4 3",
            "T|02 fa copy size=3 lo1=3",
            "T|03 s copy size=3 lo1=3",
            "T|04 m copy size=3 lo1=3",
            "T|05 fa find size=2",
            "T|06 s find size=2",
            "T|07 m find size=2",
            "T|08 fa find-inside into var size=2",
            "T|09 s find-inside into var size=2",
            "T|10 m find-inside into var size=2",
            "T|11 inside fa=1 s=1 m=1",
            "T|12 fa iq find size=3",
            "T|13 fa iq find->member size=3",
            "T|14 s iq copy size=8 '{0, 1, 2, 3, 4, 5, 6, 7}",
            "T|15 m iq find_index '{6, 7}",
            "T|16 sa q find '{2, 3, 4}",
            "T|17 concat size=9 last=100",
            "T|18 sum=28 min='{0}",
            "T|19 sorted '{7, 6, 5, 4, 3, 2, 1, 0}",
            "T|20 p fa0.iq='{'{0, 1, 2, 3, 4, 5, 6, 7, 100}, '{7, 6, 5, 4, 3, 2, 1, 0}}",
            "T|21 del size=0",
            "T|22 foreach k=0 lo=0",
            "T|22 foreach k=1 lo=3",
            "T|22 foreach k=2 lo=6",
            "T|22b s foreach k=0 lo=0",
            "T|22b s foreach k=1 lo=3",
            "T|22b s foreach k=2 lo=6",
            "T|23b s foreach 2d n=16",
            "T|23 foreach 2d n=8",
            "T|24 self find size=2",
        ]
    );
}

#[test]
fn fixed_array_of_collections_module() {
    assert_eq!(
        t_lines(include_str!("fixed_coll_module.sv")),
        [
            "T|01 size=8 p='{0, 1, 2, 3, 4, 5, 6, 7}",
            "T|02 copy size=8 p='{0, 1, 2, 3, 4, 5, 6, 7}",
            "T|03 from var size=3",
            "T|04 to var size=8",
            "T|05 find to var '{5, 6, 7}",
            "T|06 find to elem '{6, 7}",
            "T|07 concat '{0, 77}",
            "T|08 pattern '{1, 2, 3}",
            "T|09 sort '{7, 6, 5, 4, 3, 2, 1, 0}",
            "T|10 reverse '{0, 1, 2, 3, 4, 5, 6, 7}",
            "T|11 sum=28 min='{0} max='{7}",
            "T|12 all '{'{6, 7}, '{0, 1, 2, 3, 4, 5, 6, 7}, '{1, 2, 3}}",
            "T|13 foreach2 n=13",
            "T|14 foreach elem sum=28",
            "T|15 inside=1 0",
            "T|16 ins/del '{100, 1, 2, 3, 4, 5, 6, 7}",
            "T|17 pop=100 7 size=6",
            "T|18 dyn '{0, 0, 9} '{0, 0, 9}",
            "T|19 aa num=2 '{\"a\":1, \"b\":2}",
            "T|20 eq=1",
            "T|21 del size=0",
            "T|22 uniq='{1, 2, 3}",
        ]
    );
}

#[test]
fn locator_packed_struct_item_fields() {
    assert_eq!(
        t_lines(include_str!("locator_packed_item_fields.sv")),
        [
            "T|01 plain 2",
            "T|02 into var 2",
            "T|03 into elem 2",
            "T|04 into elem nofield 4",
        ]
    );
}

#[test]
fn typedef_collection_struct_members() {
    assert_eq!(
        t_lines(include_str!("typedef_coll_struct_members.sv")),
        [
            "T|01 pkg regs num=2 A=1 B=3",
            "T|02 pkg all size=4 '{16, 32, 33, 34}",
            "T|03 pkg pq '{1, 2, 3}",
            "T|04 pkg eaa A=2 '{48, 49}",
            "T|05 mod mregs A=2 at=4",
            "T|06 maa x size=2 '{5, 6} num=1",
            "T|07 mint size=3 al[1]=8",
            "T|08 mq al size=2 '{1, 2}",
            "T|09 fa al size=1",
            "T|10 daa d size=3 '{0, 0, 5}",
            "T|11 concat '{16, 32, 33, 34, 64, 65}",
            "T|12 regs '{\"A\":'{al:'{16}, at:'h1}, \"B\":'{al:'{32, 33, 34}, at:'h2}}",
            "T|13 key=A size=1",
            "T|13 key=B size=3",
            "T|14 B[0]=32",
            "T|14 B[1]=33",
            "T|14 B[2]=34",
            "T|15 B push size=4 last=35",
            "T|16 empty num=0",
            "T|17 copy-out size=2",
            "T|18 copy-in size=2 '{5, 6}",
            "T|19 find='{33, 34, 35}",
            "T|20 eq=0",
        ]
    );
}

#[test]
fn typedef_collection_members_mixed() {
    assert_eq!(
        t_lines(include_str!("typedef_coll_mixed.sv")),
        [
            "T|01 s q='{1, 2, 3} a='{\"k\":4} dsize=3",
            "T|02 n '{9} '{9}",
            "T|03 arr 2 1 '{q:'{7, 8}, a:'{\"z\":1}, d:'{}}",
            "T|04 aa num=1 q='{11}",
            "T|08 fa '{\"a\":1, \"b\":2} num=2",
            "T|09 fq '{7, 8, 9}",
            "T|10 fq idx '{1, 2}",
            "T|11 fq[0][0]=7",
            "T|11 fq[0][1]=8",
            "T|11 fq[0][2]=9",
            "T|11 fq[1][0]=1",
            "T|11 fq[1][1]=2",
            "T|12 copy '{7, 8}",
            "T|13 eq 1",
            "T|14 del 0 2",
            "T|15 sum 15",
        ]
    );
}

#[test]
fn dyn_array_concat_initializers() {
    assert_eq!(
        t_lines(include_str!("dyn_concat_initializers.sv")),
        [
            "T|01 c='{1, 2, 3, 9} d='{1, 2, 3} h='{5, 6}",
            "T|03 mc='{1, 2, 3, 9} md='{1, 2, 3}",
        ]
    );
}

#[test]
fn assoc_of_struct_delete_key() {
    assert_eq!(
        t_lines(include_str!("assoc_struct_delete.sv")),
        [
            "T|01 d=1 t=1 i=1 di=1",
            "T|02 d=0 t=0 di=0",
            "T|03 d='{\"b\":'{x:1}} di='{2:'{x:1}}",
            "T|04 d=0 di=0",
        ]
    );
}

#[test]
fn class_struct_prop_string_keyed_member() {
    let got: Vec<String> = t_lines(include_str!("class_struct_prop_colls.sv"))
        .into_iter()
        .filter(|l| !l.starts_with("T|02"))
        .collect();
    assert_eq!(got, ["T|01 '{1} '{\"x\":3}", "T|03 '{4}",]);
}

#[test]
fn pin_class_queue_push_struct_with_dyn_member() {
    assert_eq!(
        t_lines(include_str!("class_struct_prop_colls.sv")),
        ["T|01 '{1} '{\"x\":3}", "T|02 '{1}", "T|03 '{4}",]
    );
}

#[test]
fn pin_class_struct_prop_shadowed_by_module_var() {
    assert_eq!(
        t_lines(include_str!("pin_class_struct_prop_shadow.sv")),
        ["T|01 c='{1} m='{}",]
    );
}

#[test]
fn pin_class_struct_prop_whole_p() {
    assert_eq!(
        t_lines(include_str!("pin_class_struct_prop_p.sv")),
        [
            "T|01 '{q:'{1}, k:0} '{q:'{2}, k:0} '{q:'{3}, k:0}",
            "T|02 '{1} '{2} '{3}",
        ]
    );
}

#[test]
fn pin_spec_static_struct_misc() {
    assert_eq!(
        t_lines(include_str!("pin_spec_static_structs.sv")),
        [
            "T|01 show='{'{s:\"a\", v:7}}",
            "T|02 fa v=12",
            "T|03 one v=13",
            "T|04 alias v=16 s=t",
        ]
    );
}

/// Siblings of the struct-class-property pins: `push_front` and a string-
/// keyed member of a pushed element, the property copied into an element of
/// a dynamic, associative and fixed array, the property beside a same-named
/// module variable, `%p` of the whole property inside a method and from
/// outside, and a member typed by a design-level queue typedef.
#[test]
fn class_struct_prop_dyn_member_siblings() {
    assert_eq!(
        t_lines(include_str!("class_struct_prop_dyn_siblings.sv")),
        [
            "T|01 '{1} '{\"x\":3} 2",
            "T|02 '{1} '{1} '{1}",
            "T|03 c='{1} m='{}",
            "T|04 '{q:'{1}, a:'{\"x\":3}, k:5}",
            "T|05 '{q:'{1}, a:'{\"x\":3}, k:5}",
            "T|06 '{q:'{8}, k:2} '{8}",
        ]
    );
}

/// Siblings of `pin_spec_static_struct_misc`: string members of a static
/// fixed array of structs and of a static struct written from the module,
/// `%p` of the static struct in a static method, a scalar static and a
/// struct-queue element through a typedef alias, another specialization's
/// store, and a static struct member written through the alias.
#[test]
fn spec_static_struct_siblings() {
    assert_eq!(
        t_lines(include_str!("spec_static_struct_siblings.sv")),
        [
            "T|01 show='{'{s:\"a\", v:7}}",
            "T|02 fa v=12 s=z",
            "T|03 one v=13 s=o",
            "T|03b '{s:\"o\", v:13}",
            "T|04 alias v=16 s=t n=2",
            "T|05 cnt=4 4",
            "T|06 b=3 n=1",
            "T|07 one=21",
        ]
    );
}

/// §6.8, §21.2.1.7, §8.9: `%p` of a static fixed array of unpacked structs
/// prints an unwritten member at its type's default — 0 for `int` and for a
/// type parameter bound to `int`, x for `logic` — from a static method, from
/// the module through `C#(int)::fa`, for one element `N::fa[1]`, for a
/// static struct and for an instance array of a specialization.
#[test]
fn static_struct_array_p_defaults() {
    assert_eq!(
        t_lines(include_str!("static_struct_array_p_defaults.sv")),
        [
            "T|00 '{'{s:\"\", v:0}, '{s:\"\", v:0}} | '{s:\"\", v:0}",
            "T|01 '{'{s:\"\", v:0}, '{s:\"\", v:12}}",
            "T|02 '{'{s:\"\", v:0}, '{s:\"\", v:12}}",
            "T|03 '{'{s:\"\", v:0, l:x}, '{s:\"\", v:0, l:x}}",
            "T|04 '{'{s:\"\", v:5, l:x}, '{s:\"\", v:0, l:x}} '{s:\"\", v:0, l:x}",
            "T|05 '{'{v:0, b:0}, '{v:0, b:0}} '{'{v:0, b:0}, '{v:0, b:0}}",
            "T|06 '{'{v:x, b:0}, '{v:x, b:0}} '{'{v:x, b:0}, '{v:x, b:0}}",
            "T|07 '{'{v:0, b:3}, '{v:0, b:0}} 0",
        ]
    );
}
