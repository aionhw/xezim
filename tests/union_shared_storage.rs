//! IEEE 1800-2017 §7.3: "A union is a data type that represents a single piece
//! of storage that can be accessed using one of its named member data
//! identifiers." Writing one member must therefore be readable through any
//! other.
//!
//! PACKED unions already aliased (one signal, members are bit-slices).
//! UNPACKED ones gave every member its OWN signal, so `u.raw_crc` read `x`
//! after `u.packet_id` was written. Give an untagged union a single signal with
//! all members at bit 0, exactly like a packed one.
//!
//! Two things fell out of that. A nested union's dotted name (`s.meta.crc`)
//! parses as ONE hierarchical identifier, and name resolution used to collapse
//! it to its last segment once the leaf signal stopped existing. And `%p` on a
//! bit-slice lost the member's signedness.
//!
//! TAGGED unions are tag-checked (§7.3.2) and keep their own storage.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef union { int packet_id; bit [31:0] raw_crc; } u_t;
  typedef union packed { bit [31:0] a; bit [31:0] b; } up_t;
  typedef struct { int cid; u_t meta; } node_t;

  u_t    u;
  up_t   p;
  node_t s;
  node_t arr[2];

  int  u_pid, u_crc;
  int  p_a, p_b;
  int  s_pid, s_crc;
  int  a0_crc, a1_pid;

  initial begin
    // Top-level unpacked union.
    u.packet_id = 12345;
    u_pid = u.packet_id;
    u_crc = u.raw_crc;

    // Packed union (regression guard).
    p.a = 32'hDEADBEEF;
    p_a = p.a;
    p_b = p.b;

    // Union nested inside an unpacked struct.
    s.cid = 1;
    s.meta.packet_id = 777;
    s_pid = s.meta.packet_id;
    s_crc = s.meta.raw_crc;

    // Union nested inside a struct inside an unpacked array.
    arr[0].meta.packet_id = 42;
    a0_crc = arr[0].meta.raw_crc;
    arr[1].meta.raw_crc = 99;
    a1_pid = arr[1].meta.packet_id;

    // %p on a union prints every member off the shared storage, and a
    // signed member prints signed.
    $display("U=%p", u);
    $display("S=%p", s);
  end
endmodule
"#;

fn u32v(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn unpacked_union_members_share_one_storage() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u32v(&sim, "u_pid"), 12345);
    assert_eq!(u32v(&sim, "u_crc"), 12345, "u.raw_crc must alias u.packet_id");
}

#[test]
fn packed_union_still_aliases() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u32v(&sim, "p_a"), 0xDEAD_BEEF);
    assert_eq!(u32v(&sim, "p_b"), 0xDEAD_BEEF);
}

#[test]
fn union_nested_in_struct_and_array_aliases() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // `s.meta.raw_crc` resolves as a whole dotted path, not as `raw_crc`.
    assert_eq!(u32v(&sim, "s_pid"), 777);
    assert_eq!(u32v(&sim, "s_crc"), 777, "s.meta.raw_crc must alias s.meta.packet_id");

    // Independent storage per array element, aliasing within each.
    assert_eq!(u32v(&sim, "a0_crc"), 42);
    assert_eq!(u32v(&sim, "a1_pid"), 99);
}

#[test]
fn p_format_prints_union_members_off_shared_storage() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(line(&sim, "U="), "U='{packet_id:12345, raw_crc:12345}");
    assert_eq!(line(&sim, "S="), "S='{cid:1, meta:'{packet_id:777, raw_crc:777}}");
}

/// A bit-slice must keep the member's declared signedness: an `int` union
/// member holding 32'hDEADBEEF prints -559038737, an unsigned one 3735928559.
const SIGNED: &str = r#"
module tb;
  typedef union { int signed_id; bit [31:0] unsigned_crc; } u_t;
  u_t u;
  initial begin
    u.unsigned_crc = 32'hDEADBEEF;
    $display("U=%p", u);
  end
endmodule
"#;

#[test]
fn p_format_keeps_member_signedness_across_a_slice() {
    let sim = simulate(SIGNED, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "U="),
        "U='{signed_id:-559038737, unsigned_crc:3735928559}"
    );
}
