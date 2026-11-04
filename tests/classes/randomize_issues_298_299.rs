//! `randomize` on dynamic arrays whose sizes are tied to other variables
//! (issues #298 and #299, reported by Ganesh-TNL).
//!
//! - #298, §18.5.8.1/§18.5.10: a size constraint is solved before the
//!   elements, jointly with the variables it reads (`beats.size() ==
//!   cmd.count + 1`); the elements follow at that size. The arrays were
//!   sized from the trial's draw of `cmd.count`, and the joint solver then
//!   had to pin the count to it. Struct elements typed by a class type
//!   parameter were not modelled at all, a variable split into fields was
//!   decided low bits first, and a relation the evaluator judges waited
//!   for every field of the struct.
//! - #299, §18.12: several dynamic arrays with `foreach` constraints in one
//!   `std::randomize`; their sizes are solved first there too.
//!
//! Expected values come from the reference simulator. Random values differ
//! between simulators, so the probes print whether the constraints hold.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
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

fn assert_pass(out: &[String]) {
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "no TEST_PASS: {out:?}"
    );
    assert!(
        !out.iter().any(|m| m.contains("FAIL")),
        "failure reported: {out:?}"
    );
}

#[test]
fn issue_298_size_coupled_struct_arrays() {
    let out = lines(include_str!("randomize_issue_298_size_coupled.sv"));
    assert_pass(&out);
    assert!(
        out.iter().any(
            |m| m.starts_with("OK t1 randomized with: ") && m.ends_with("sz=2 count=0 beats=1")
        ),
        "{out:?}"
    );
    assert!(out.iter().any(|m| m.starts_with("OK t2 randomized: ")));
}

#[test]
fn issue_299_several_arrays_with_foreach() {
    let out = lines(include_str!("randomize_issue_299_multi_array.sv"));
    assert_pass(&out);
    assert!(
        out.iter()
            .any(|m| m == "OK t1 two-array: size=37 p[3]=3 q[3]=3")
    );
    assert!(out.iter().any(|m| m.starts_with("OK t2 sizes 37 272 op=")));
}

/// Sizes tied to scalars, to each other and through a sum; struct arrays
/// with a dist; nested `foreach`; elements of a type-parameter type.
#[test]
fn size_phase_class_shapes() {
    let expected: Vec<String> = (1..=7).map(|k| format!("T|a{k} fails=0 bad=0")).collect();
    assert_eq!(
        t_lines(include_str!("randomize_size_phase_class.sv")),
        expected
    );
}

/// `std::randomize` over two to four arrays in a task, a module block and
/// a class; sizes from a scalar target; coupled sizes with a sum; nested
/// `foreach`; queues.
#[test]
fn size_phase_scope_shapes() {
    let expected = [
        "T|t1 task two arrays fails=0 bad=0",
        "T|t2 inline four arrays fails=0 bad=0",
        "T|t3 sizes from scalar target fails=0 bad=0",
        "T|t4 sizes coupled + sum fails=0 bad=0",
        "T|t5 class two arrays fails=0 bad=0",
        "T|t6c class three arrays sized by n fails=0 bad=0",
        "T|t6 nested foreach fails=0 bad=0",
        "T|t8 two queues fails=0 bad=0",
    ];
    assert_eq!(
        t_lines(include_str!("randomize_size_phase_scope.sv")),
        expected
    );
}

/// A soft size against a hard bound, sizes tied to each other, a size
/// range read from a scalar, an open size, sizes under implications, a
/// scalar computed from a size, a `std::randomize` sum of sizes.
#[test]
fn size_phase_edges() {
    let expected: Vec<String> = ["e1", "e3", "e4", "e5", "e6", "e7", "s1"]
        .iter()
        .map(|k| format!("T|{k} fails=0 bad=0"))
        .collect();
    assert_eq!(
        t_lines(include_str!("randomize_size_phase_edges.sv")),
        expected
    );
}

/// The #298 class's scalar distribution over 100 draws; its empty beat
/// arrays remain state with rand_mode off. The reference gives
/// `allow_cfg_space` = 1 in all of them and 14 counts below 32.
#[test]
fn issue_298_distribution() {
    let out = t_lines(include_str!("randomize_issue_298_distribution.sv"));
    let [line] = out.as_slice() else {
        panic!("unexpected output: {out:?}");
    };
    let num = |key: &str| -> i64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(key))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {key} in {line}"))
    };
    assert_eq!(num("T|fails="), 0, "{line}");
    assert_eq!(num("bad="), 0, "{line}");
    assert!(num("acs=") >= 90, "{line}");
    assert!((5..=30).contains(&num("low=")), "{line}");
}

/// §7.5.1/§7.10.2: `size()` and `num()` are ints in a comparison.
#[test]
fn size_method_compares_at_int_width() {
    let expected = [
        "T|method 1 0",
        "T|module 1 1 1",
        "T|handle 1 1",
        "T|assign 1 func 1",
        "T|task 1",
        "T|narrow 1",
    ];
    assert_eq!(
        t_lines(include_str!("size_method_compare_width.sv")),
        expected
    );
}

/// §7.2.1/§8.25: field writes and reads on class arrays of packed structs.
#[test]
fn class_struct_array_field_writes() {
    let expected = [
        "T|in w 1 1234 u 1 1234 s 05 f 06 12 one 03",
        "T|out w 1 1234 1 u 1 1234 beef s 30 f 17",
        "T|bits 146 16",
    ];
    assert_eq!(
        t_lines(include_str!("class_struct_array_field_writes.sv")),
        expected
    );
}
