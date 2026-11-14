//! IEEE 1800-2023 regression and main-passing guards for upstream #1099.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct RunDir(std::path::PathBuf);
impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check(source: &str, expected: &[&str], no_warnings: bool) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = RunDir(std::env::temp_dir().join(format!(
        "xezim-1099-{}-{unique}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    std::fs::write(dir.0.join("test.sv"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(&dir.0)
        .args(["--no-cache", "test.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    if no_warnings {
        assert!(!stderr.to_ascii_lowercase().contains("warning"), "{stderr}");
    }
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("T|") || line.starts_with("PASS") || line.starts_with("FAIL")
        })
        .collect();
    // Several independent initial blocks may print at the same time. Compare
    // the multiset so the test does not impose an Active-region ordering (§4.7).
    let mut actual = lines;
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}\n{stderr}");
}

/// §7.2: "the individual data types that make up the structure can be referenced by name"; §§27.3, 23.6 give independent generated declarations and hierarchical access; §10.3.2 copies 5.0.
#[test]
fn generated_real_members_have_independent_storage() {
    check(
        include_str!(
            "generate_real_struct_members_1099/generated_real_members_have_independent_storage.sv"
        ),
        &[
            "PASS inside a=1.0 b=2.0",
            "PASS outside a=1.0 b=2.0 only=5.0",
            "PASS copy=5.0",
        ],
        false,
    );
}

/// §23.6: "shall have a unique hierarchical path name"; §6.8 default four-state variable is x; §6.7.1: net default "shall be the value z".
#[test]
fn keeps_namesakes_in_module_child_and_generate_scopes() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_namesakes_in_module_child_and_generate_scopes.sv"
        ),
        &["T|generated xx", "T|parent zz"],
        false,
    );
}

/// §6.16 string stores the assigned characters; §6.16: "The string data type is an ordered collection of characters"; §21.2.1.7 formats the string characters without packed-vector padding. Expected string lengths are 5 and 3.
#[test]
fn keeps_hierarchical_string_formatting() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_hierarchical_string_formatting.sv"),
        &["T|string [hello] [hello] 5", "T|string [bye] [bye] 3"],
        false,
    );
}

/// §7.9.1: num() returns "the number of entries in the associative array"; two distinct string keys give 2.
#[test]
fn keeps_associative_member_entry_count() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_associative_member_entry_count.sv"),
        &["T|entries 2", "T|module-entries 2"],
        false,
    );
}

/// §10.6.2: a force "shall override all drivers of the net"; §23.3.3 inout is a "non-strength-reducing transistor connection"; §7.2 member values are the forced pattern bytes.
#[test]
fn keeps_whole_structure_force_through_inout() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_whole_structure_force_through_inout.sv"
        ),
        &["T|force ab cd"],
        false,
    );
}

/// §6.6.6 supply nets have supply strength; §28.12.1: "the stronger signal shall dominate all the weaker drivers". Supply0 remains 0000 and supply1 1111.
#[test]
fn keeps_supply_structure_members() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_supply_structure_members.sv"),
        &["T|supply 0000 1111", "T|floating 0000 1111"],
        false,
    );
}

/// §6.12: "The shortreal data type is the same as a C float"; §20.5 uses 32-bit IEEE representation, whose spacing here is 2, so 16777216.25 rounds to 16777216.0.
#[test]
fn keeps_shortreal_member_rounding() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_shortreal_member_rounding.sv"),
        &["T|outside 16777216.00 16777216.00 delta 0.00 0.00"],
        false,
    );
}

/// §§6.12,20.5 binary32/64 exactly represent 1023*2^52=4607182418800017408; §6.18 preserves typedef identity; §5.6.1 escaped identifiers include printable non-whitespace characters. §6.12: "The shortreal data type is the same as a C float."
#[test]
fn keeps_declined_shortreal_fallback() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_declined_shortreal_fallback.sv"),
        &["T|short 4607182418800017408.0 reference 4607182418800017408.0"],
        false,
    );
}

/// §§6.12,20.5 binary32/64 exactly represent 1023*2^52=4607182418800017408; §6.18 preserves typedef identity; §5.6.1 escaped identifiers include printable non-whitespace characters. §6.12: "The shortreal data type is the same as a C float."
#[test]
fn keeps_escaped_shortreal_fallback() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_escaped_shortreal_fallback.sv"),
        &["T|short 4607182418800017408.0 reference 4607182418800017408.0"],
        false,
    );
}

/// §6.17 initializes a new event object; §15.5.1: named event triggers "unblock all processes currently waiting"; §15.5.2 waiter starts before the #1 trigger, giving seen=1 at #2.
#[test]
fn keeps_event_member_waiter_completion() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_event_member_waiter_completion.sv"),
        &["T|seen 1"],
        false,
    );
}

/// §§7.2,6.19 permit packed dimensions after bodies; §7.4.1: packed array is "represented as a contiguous set of bits"; §20.9 $isunknown detects preserved x bits in the high subarray.
#[test]
fn keeps_packed_struct_and_enum_body_select_unknowns() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_packed_struct_and_enum_body_select_unknowns.sv"
        ),
        &["T|struct-unknown 1", "T|enum-unknown 1"],
        false,
    );
}

/// §6.7.1: net default "shall be the value z"; §6.6.5 gives tri0/tri1 continuous pull drivers; §23.3.3 net output does not contribute a variable x driver.
#[test]
fn keeps_undriven_structure_output_defaults() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_undriven_structure_output_defaults.sv"
        ),
        &["T|idle zz 00 ff"],
        false,
    );
}

/// §§7.2,10.9,6.24.1 preserve z in patterns/casts; §11.4.11 selects off; §6.6.5 supplies pull 1; §28.12.1 pull beats weak 0, giving zz/ff/ff/ff/ff. §6.6.5: "A tri1 net is equivalent to a wire net with a continuous 1 value of pull strength driving it."
#[test]
fn keeps_z_expression_and_strength_wrapped_structure_drivers() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_z_expression_and_strength_wrapped_structure_drivers.sv"
        ),
        &["T|off zz ff ff ff ff"],
        false,
    );
}

/// §6.6.5 tri1 has "a continuous 1 value of pull strength driving it"; §§28.3.2,28.10 set weak primitive outputs; §28.12.1 stronger pull 1 dominates weak 0.
#[test]
fn keeps_weak_primitive_structure_pulls() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_weak_primitive_structure_pulls.sv"),
        &["T|child 1", "T|generated 1", "T|primitive 1 1"],
        false,
    );
}

/// §6.7.1 permits fixed arrays of valid net members; §6.6.5 preserves implicit pulls against z; §20.9 $isunknown is false for fully resolved 0/1. §6.6.5: "A tri1 net is equivalent to a wire net with a continuous 1 value of pull strength driving it."
#[test]
fn keeps_fixed_array_member_pull_bits() {
    check(
        include_str!("generate_real_struct_members_1099/keeps_fixed_array_member_pull_bits.sv"),
        &["T|array-pulls 0 1 unknown 0 0"],
        false,
    );
}

/// §7.2: a member may have "a default value" (module values 1.25,7); §§7.4.2,10.5,23.6 give direct element reads the explicitly assigned 2.5/2 and 4.5/4.
#[test]
fn keeps_explicit_defaults_and_direct_array_reads() {
    check(
        include_str!(
            "generate_real_struct_members_1099/keeps_explicit_defaults_and_direct_array_reads.sv"
        ),
        &[
            "T|defaults 1.25 7",
            "T|explicit 1.25 7",
            "T|arrays 2.5 2 4.5 4",
        ],
        false,
    );
}
