//! #925 (reported by Ganesh-TNL): a COLLECTION of virtual interfaces passed
//! as a subroutine or constructor argument (`virtual X f[]`, `f[$]`,
//! `f[2]`, `f[2][2]`), or declared as a task local, lost its element
//! handles: `f[0]` compared equal to null, `f[0].rst_l` read x and
//! `@(posedge f[0].clk)` never armed. The elements live under the frame's
//! per-call store (`formal_dyn_key` / `declare_local_dyn`) and a formal
//! hides a same-named property of `this` (IEEE 1800-2023 §13.5.1, §23.9,
//! §25.9); the bare-name resolution looked only at module variables and
//! `this` properties. A 2-D vif array element (`v2[0][1]`) was not
//! resolved at all (§7.4.5, §25.10).
//!
//! Expected values from the reference simulator: every check passes.
use xezim::simulate;

fn passes(src: &str) -> (u64, Vec<String>) {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    let failures = sim
        .get_signal("failures")
        .or_else(|| sim.get_signal("testbench.failures"))
        .unwrap_or_else(|| panic!("signal not found: failures"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    (failures, out)
}

/// The reporter's reproducer: groups A (constructor formal), D (function
/// and task formals, event control on a formal element), E (2-D property,
/// task-local fixed and dynamic arrays) fail on main; B, C and F are the
/// regression guard. Outcome codes: 1 fired, 2 stuck, 3 spin.
#[test]
fn vif_collection_formals_925() {
    let (failures, out) = passes(include_str!("vif_collection_formals_925.sv"));
    let fails: Vec<&String> = out.iter().filter(|m| m.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "FAIL lines: {:#?}", fails);
    assert_eq!(failures, 0);
    assert!(out.iter().any(|m| m == "TEST_PASS"), "TEST_PASS must print");
}

/// Neighbouring shapes: `ref` and `output` array formals, a fixed-size
/// formal with `@(posedge f[0].clk)`, `@(f[0].cb)` and `wait (f[0].go)`, a
/// 2-D formal, a queue formal grown inside the callee, an assoc formal,
/// task-local queue / assoc / fixed arrays (with `foreach`), a struct member
/// holding a vif, a vif array returned from a function and a typedef'd vif
/// array formal.
#[test]
fn vif_collection_formals_925_probes() {
    let (failures, out) = passes(include_str!("vif_collection_formals_925_probes.sv"));
    let t: Vec<&String> = out.iter().filter(|m| m.starts_with("T|")).collect();
    let fails: Vec<&&String> = t.iter().filter(|m| !m.starts_with("T|PASS")).collect();
    assert!(
        fails.len() == 1 && fails[0].as_str() == "T|TEST_PASS",
        "unexpected T| lines: {:#?}",
        fails
    );
    assert_eq!(t.len(), 19, "every probe must report: {:#?}", t);
    assert_eq!(failures, 0);
}
