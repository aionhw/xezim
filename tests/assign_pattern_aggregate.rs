//! Assignment patterns written to aggregates whose leaves live in SEPARATE
//! signals — unpacked structs, unpacked arrays of them, and associative arrays
//! (IEEE 1800-2017 §10.9.2, §10.10).
//!
//! These were collapsed into a single packed value and written to a container
//! signal nobody reads, so every member stayed X and an associative array
//! stayed empty. Because elaboration lowers a declaration initializer
//! `T v[int] = '{...}` to `v = '{...}` in an initial block, this also fixes
//! declaration initializers — including the LRM's `combo_t cmb[int]` example.
//!
//! Packed structs and arrays of scalars must keep the packed path.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef enum { ON, OFF } toggle_e;
  typedef struct { toggle_e tgl; string str; } combo_t;
  typedef struct packed { bit [3:0] a; bit [3:0] b; } pk_t;

  combo_t u_ord, u_named;
  pk_t    pk;
  combo_t arr[2];
  combo_t cs[int];

  // Declaration initializers.
  int     mi[int]    = '{10:100, 20:200};
  int     ms[string] = '{"a":1, "b":2};
  combo_t cmb[int]   = '{10:'{OFF, "toggle10"}, 20:'{ON, "toggle20"}};
  int     fixed[3]   = '{1, 2, 3};

  int mi_num;
  int pk_a, pk_b;
  int cmb20_tgl;

  initial begin
    u_ord   = '{OFF, "x"};
    u_named = '{tgl: ON, str: "longer-string-value"};
    pk      = '{a: 4'h3, b: 4'h5};
    arr     = '{'{ON, "a0"}, '{OFF, "a1"}};
    cs[5]   = '{OFF, "five"};

    mi_num    = mi.num();
    pk_a      = pk.a;
    pk_b      = pk.b;
    cmb20_tgl = cmb[20].tgl;

    $display("UORD=%p", u_ord);
    $display("UNAMED=%p", u_named);
    $display("ARR=%p", arr);
    $display("CS=%p", cs);
    $display("MI=%p", mi);
    $display("MS=%p", ms);
    $display("CMB=%p", cmb);
    $display("FIXED=%p", fixed);
    $display("CMB10STR=%0s", cmb[10].str);
  end
endmodule
"#;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn pattern_assign_spreads_into_unpacked_struct() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Ordered items bind positionally; named items by member name. A string
    // member must keep its full text (no truncation to the container width).
    assert_eq!(line(&sim, "UORD="), r#"UORD='{tgl:OFF, str:"x"}"#);
    assert_eq!(
        line(&sim, "UNAMED="),
        r#"UNAMED='{tgl:ON, str:"longer-string-value"}"#
    );
}

#[test]
fn pattern_assign_spreads_into_array_of_structs_and_assoc_element() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    assert_eq!(
        line(&sim, "ARR="),
        r#"ARR='{'{tgl:ON, str:"a0"}, '{tgl:OFF, str:"a1"}}"#
    );
    // `assoc[key] = '{...}` on a struct element.
    assert_eq!(line(&sim, "CS="), r#"CS='{5:'{tgl:OFF, str:"five"}}"#);
}

#[test]
fn assoc_declaration_initializers_populate_elements() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    assert_eq!(line(&sim, "MI="), "MI='{10:100, 20:200}");
    assert_eq!(line(&sim, "MS="), r#"MS='{"a":1, "b":2}"#);
    // The LRM §21.2.1.7 example: struct elements keyed by integer.
    assert_eq!(
        line(&sim, "CMB="),
        r#"CMB='{10:'{tgl:OFF, str:"toggle10"}, 20:'{tgl:ON, str:"toggle20"}}"#
    );

    // Populated for real, not just printable: num() counts them, a member
    // reads back, and an element member is addressable by name.
    assert_eq!(u(&sim, "mi_num"), 2, "assoc decl init did not populate");
    assert_eq!(u(&sim, "cmb20_tgl"), 0, "cmb[20].tgl should be ON(0)");
    assert_eq!(line(&sim, "CMB10STR="), "CMB10STR=toggle10");
}

#[test]
fn packed_struct_and_scalar_array_keep_the_packed_path() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // A packed struct is ONE signal — the pattern must still collapse.
    assert_eq!(u(&sim, "pk_a"), 0x3);
    assert_eq!(u(&sim, "pk_b"), 0x5);
    // A fixed array of scalars keeps its ordered element init.
    assert_eq!(line(&sim, "FIXED="), "FIXED='{1, 2, 3}");
}

/// An unpacked array member of SCALAR element type (`real m[3]`) is spread the
/// same way — it is not a struct, so it must not be rejected for lacking one.
const REALARR: &str = r#"
module tb;
  typedef struct { int idx; real mm[3]; } rec_t;
  rec_t a[2];
  real  top[3];
  initial begin
    a[0].idx = 0; a[0].mm = '{1.5, 2.5, 3.5};
    a[1].idx = 1; a[1].mm = '{4.0, 5.0, 6.0};
    top = '{9.5, 8.25, 7.125};
    $display("A=%p", a);
    $display("TOP=%p", top);
  end
endmodule
"#;

#[test]
fn pattern_assign_spreads_into_real_array_members() {
    let sim = simulate(REALARR, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "A="),
        "A='{'{idx:0, mm:'{1.5, 2.5, 3.5}}, '{idx:1, mm:'{4, 5, 6}}}"
    );
    assert_eq!(line(&sim, "TOP="), "TOP='{9.5, 8.25, 7.125}");
}

/// A pattern written to a nested sub-path, and `%p` read back from one. A
/// sub-path has no declaration of its own, so both directions must derive the
/// type by walking the base variable's declared type.
const NESTED: &str = r#"
module tb;
  typedef enum { ON, OFF } toggle_e;
  typedef struct { toggle_e tgl; string str; } combo_t;
  typedef struct { int cid; combo_t nodes[2]; } cl_t;

  cl_t    c, c2;
  combo_t arr[2];
  combo_t cmb[int] = '{10:'{OFF, "toggle10"}, 20:'{ON, "toggle20"}};

  initial begin
    c.cid      = 7;
    c.nodes[0] = '{OFF, "n0"};                        // element of array member
    c.nodes[1] = '{tgl: ON, str: "n1"};
    c2         = '{cid: 9, nodes: '{'{ON, "z0"}, '{OFF, "z1"}}};  // whole nested
    arr[0]     = '{ON, "a0"};
    arr[1]     = '{OFF, "a1"};

    $display("C=%p", c);
    $display("C2=%p", c2);
    $display("CN=%p", c.nodes);        // unindexed array member
    $display("CN0=%p", c.nodes[0]);    // element of array member
    $display("E0=%p", arr[0]);         // element of a top-level array
    $display("E0STR=%p", arr[0].str);  // member of an element
    $display("K20=%p", cmb[20]);       // associative element
    $display("K10STR=%p", cmb[10].str);
  end
endmodule
"#;

#[test]
fn pattern_assign_and_p_format_reach_nested_sub_paths() {
    let sim = simulate(NESTED, 100).expect("simulate failed");

    // Writes: element of an array member, and a whole struct whose member is
    // itself an unpacked array of structs.
    assert_eq!(
        line(&sim, "C="),
        r#"C='{cid:7, nodes:'{'{tgl:OFF, str:"n0"}, '{tgl:ON, str:"n1"}}}"#
    );
    assert_eq!(
        line(&sim, "C2="),
        r#"C2='{cid:9, nodes:'{'{tgl:ON, str:"z0"}, '{tgl:OFF, str:"z1"}}}"#
    );

    // Reads: `%p` on sub-paths, which previously printed `x` or raw decimal.
    assert_eq!(
        line(&sim, "CN="),
        r#"CN='{'{tgl:OFF, str:"n0"}, '{tgl:ON, str:"n1"}}"#
    );
    assert_eq!(line(&sim, "CN0="), r#"CN0='{tgl:OFF, str:"n0"}"#);
    assert_eq!(line(&sim, "E0="), r#"E0='{tgl:ON, str:"a0"}"#);
    assert_eq!(line(&sim, "E0STR="), r#"E0STR="a0""#);
    assert_eq!(line(&sim, "K20="), r#"K20='{tgl:ON, str:"toggle20"}"#);
    assert_eq!(line(&sim, "K10STR="), r#"K10STR="toggle10""#);
}
