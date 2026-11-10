//! #944 (reported by Ganesh-TNL): a base class owns an instance property `x`
//! and a derived class redeclares the name as a `localparam`, `parameter`,
//! `static` property or typedef. IEEE 1800-2023 §8.14/§8.23: a name used in
//! the base's methods (`this.x`, bare `x`, `x += 1`, `this.x[3:0]`, a `ref`
//! actual, a fork, a nonblocking write, a constraint of the base) resolves in
//! the base's scope, and a name reached through a reference (`h.x`,
//! `q[0].x`, `aa["k"].x`, `Box::slot.x`, `this.child.x`) in the scope of the
//! reference's declared class. The object's own class decided instead, so
//! every such write went to the derived class's static cell or constant and
//! the base's property never changed. The inverse shape (a static of the
//! base hidden by a derived instance property) and an instance property
//! redeclared as an instance property follow the same rule.
//!
//! Expected values from the reference simulator. The reproducer's G23 used
//! `mk_base().prop_val = 31`, which is no variable_lvalue (Annex A.8.5) and
//! which the reference simulator rejects at run time; the test calls a
//! method on the function result instead.
use xezim::simulate;

fn run(src: &str, max_time: u64) -> (Option<u64>, Vec<String>) {
    let sim = simulate(src, max_time).expect("simulate failed");
    let failures = sim
        .get_signal("failures")
        .or_else(|| sim.get_signal("tb_top.failures"))
        .and_then(|v| v.to_u64());
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    (failures, out)
}

fn t_lines(out: &[String]) -> Vec<&str> {
    out.iter()
        .filter(|m| m.starts_with("T|"))
        .map(|m| m.as_str())
        .collect()
}

/// The reporter's self-checking file: every group G0..G43 passes.
#[test]
fn static_shadow_write_944() {
    let (failures, out) = run(include_str!("static_shadow_write_944.sv"), 100_000);
    let fails: Vec<&String> = out.iter().filter(|m| m.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "FAIL lines: {:#?}", fails);
    assert!(
        out.iter().any(|m| m == "TEST_PASS"),
        "no TEST_PASS: {:#?}",
        out
    );
    assert_eq!(failures, Some(0));
}

/// Through `this`, a base-typed and a derived-typed reference, in a derived
/// method, in a grandchild, the inverse shape (a static of the base hidden by
/// a derived instance property) and inline constraints.
#[test]
fn static_shadow_write_944_scopes() {
    let (_, out) = run(include_str!("static_shadow_write_944_probes_a.sv"), 100);
    assert_eq!(
        t_lines(&out),
        [
            "T|N1 ds.x=27",
            "T|N2 ds.x=5 DS::x=5 get=0",
            "T|N3 dl.d_get=27 d_get_this=27 d_get_super=0 ds.d_get=27 ds.d_get_this=27",
            "T|N4 dq[0].x=27",
            "T|N5 DS::x=8 get=0",
            "T|N6 DS::x=9 e.get=0 e_get=9",
            "T|N7 b.x=3 get_this=3",
            "T|N8 A2::y=11 a_get=11 b_get=12 b2.y=12",
            "T|N8b A2::y=13 b_get=12 a2.y=13",
            "T|N9 ok=1 get=15",
            "T|N9b ok=0 get=15",
        ]
    );
}

/// References that are no variable: queue and associative-array elements,
/// a static handle, a member handle, a member queue element, a `foreach`
/// over handles; each declared base- and derived-typed.
#[test]
fn static_shadow_write_944_references() {
    let (_, out) = run(include_str!("static_shadow_write_944_probes_b.sv"), 100);
    assert_eq!(
        t_lines(&out),
        [
            "T|R1 qb[0].x=0 ab=0 qd[0].x=27 ad=27",
            "T|W1 ds.get=3 DS::x=27",
            "T|W2 ds2.get=0 DS::x=4",
            "T|W3 ds.get=5 ds2.get=0 DS::x=6",
            "T|W4 ds.get=7 ds2.get=0 DS::x=8 sb.x=7",
            "T|W5 ds2.get=9 DS::x=27 get_child=9",
            "T|W6 ds2.get=9 DS::x=10 get_dchild=10",
            "T|W7 ds2.get=11 DS::x=27 get_pool=11",
            "T|W8 ds.get=12 ds2.get=11 DS::x=13",
            "T|W9 dl.get=14 get_child=14",
        ]
    );
}

/// §8.14/§8.10: an instance property redeclared as an instance property,
/// written through a queue element and a member chain of base type.
#[test]
fn static_shadow_write_944_instance_redeclared() {
    let (_, out) = run(include_str!("static_shadow_write_944_probes_c.sv"), 100);
    assert_eq!(
        t_lines(&out),
        [
            "T|I1 b.x=5 d.x=0 bget=5 dget=0",
            "T|I2 b.x=5 d.x=6 bget=5 dget=6",
            "T|I3 q0.x=7 bget=7 dget=6",
            "T|I4 nxt.x=8 bget=8 dget=0",
        ]
    );
}
