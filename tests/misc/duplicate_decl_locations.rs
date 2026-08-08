//! A duplicate-declaration error must name BOTH declarations — where the
//! first one is and where the offending one is.
//!
//! Two things made that impossible before:
//!
//!   * Nothing recorded where a name was FIRST declared, so the error could
//!     only ever point at the duplicate. In a large multi-file design that
//!     leaves the user grepping for the other one.
//!
//!   * `span_location` resolved a span by scanning every retained source for
//!     one whose length could contain the offset, and gave up as "ambiguous"
//!     if more than one could. A `Span` is a byte offset into ITS OWN file, so
//!     with several files almost every offset fits in more than one of them —
//!     the location silently vanished for any multi-file design, which is
//!     every real one. It now prefers the file that defines the module being
//!     elaborated (`src_file_of_module`, published through the same
//!     thread-local as the sources because the module's own copy is assigned
//!     only after elaboration returns).

/// Elaborate `files` (display name, source) and return the error text, failing
/// if it unexpectedly elaborates cleanly. The names are only labels for the
/// diagnostic — nothing is read back off disk.
fn elab_err(files: &[(&str, &str)]) -> String {
    let sources: Vec<String> = files.iter().map(|(_, s)| (*s).to_string()).collect();
    let paths: Vec<String> = files.iter().map(|(n, _)| (*n).to_string()).collect();
    let res = xezim::simulate_multi(
        &sources,
        10,
        xezim::SimOptions {
            source_paths: paths.to_vec(),
            ..Default::default()
        },
    );
    match res {
        Ok(_) => panic!("expected a duplicate-declaration error, but it elaborated"),
        Err(e) => e,
    }
}

/// The reported case: three files, the duplicate in the last one. Both lines
/// must be named. Before the fix neither location appeared at all.
#[test]
fn duplicate_parameter_reports_both_locations_across_files() {
    let err = elab_err(&[
        ("a_pkg.sv", "package cfg_pkg;\n  parameter int UNUSED_A = 1;\n  parameter int UNUSED_B = 2;\nendpackage\n"),
        ("b_defs.sv", "package other_pkg;\n  parameter int SOMETHING = 3;\nendpackage\n"),
        (
            "c_bfm.sv",
            "module fc_dram_bfm;\n  parameter int CONFIG = 5;\n  localparam int CONFIG = 9;\n  initial $display(\"%0d\", CONFIG);\nendmodule\n",
        ),
    ]);
    assert!(err.contains("duplicate declaration of 'CONFIG'"), "got: {err}");
    assert!(
        err.contains("first declaration (parameter/localparam) is at"),
        "must point at the FIRST declaration; got: {err}"
    );
    assert!(
        err.contains("c_bfm.sv:2"),
        "first declaration is on line 2 of the third file; got: {err}"
    );
    assert!(
        err.contains("c_bfm.sv:3"),
        "the duplicate is on line 3 of the third file; got: {err}"
    );
}

/// A conflicting variable re-declaration takes a different path into the same
/// error (`note_explicit_type`), so it is pinned separately.
#[test]
fn duplicate_variable_reports_both_locations() {
    let err = elab_err(&[(
        "m.sv",
        "module dut4;\n  int  thing;\n  real thing;\n  initial $display(\"%0d\", thing);\nendmodule\n",
    )]);
    assert!(err.contains("duplicate declaration of 'thing'"), "got: {err}");
    assert!(
        err.contains("first declaration (variable/net) is at"),
        "must point at the FIRST declaration; got: {err}"
    );
    assert!(err.contains("m.sv:2"), "first declaration line; got: {err}");
    assert!(err.contains("m.sv:3"), "duplicate line; got: {err}");
}

/// The existing context lines must survive — they carry what the name already
/// IS, which is usually the actual surprise.
#[test]
fn duplicate_error_keeps_its_context_lines() {
    let err = elab_err(&[(
        "m.sv",
        "module dut5;\n  parameter int W = 4;\n  localparam int W = 8;\nendmodule\n",
    )]);
    assert!(
        err.contains("already declared as a parameter/localparam"),
        "got: {err}"
    );
    assert!(err.contains("while elaborating 'dut5'"), "got: {err}");
    assert!(err.contains("--no-strict"), "got: {err}");
}

/// §26.3 — a package's contents are only in scope where the package is
/// IMPORTED. xezim registers them design-wide under their bare name, so a
/// module that never imported the package still saw them and rejected an
/// ordinary local declaration whose name happened to match an enum member of
/// some unrelated package elsewhere in the design. Every other simulator
/// accepts this; it blocked a real build.
#[test]
fn local_declaration_beats_an_unimported_packages_enum_member() {
    let sources = [
        "package state_types_pkg;\n  typedef enum logic [1:0] { CONFIG = 2'b00, XFER = 2'b01, ERR = 2'b10 } state_ctrl_e;\nendpackage\n".to_string(),
        "module fc_dram_bfm;\n  int CONFIG = 5;\n  int seen;\n  initial seen = CONFIG;\nendmodule\n".to_string(),
    ];
    let paths = ["pkg.sv".to_string(), "bfm.sv".to_string()];
    let sim = xezim::simulate_multi(
        &sources,
        10,
        xezim::SimOptions {
            source_paths: paths.to_vec(),
            ..Default::default()
        },
    )
    .expect("a local declaration must not collide with an unimported package's enum member");
    let v = sim
        .get_signal("seen")
        .or_else(|| sim.get_signal("fc_dram_bfm.seen"))
        .expect("signal 'seen' not found")
        .to_u64()
        .expect("not u64-able");
    assert_eq!(v, 5, "the LOCAL declaration must win, not the enum member");
}

/// The displacement above is worth exactly one declaration. A module that
/// declares the same name TWICE is still an ordinary duplicate, and the error
/// must blame the local declaration rather than the package member.
#[test]
fn unimported_package_name_declared_twice_is_still_a_duplicate() {
    let err = elab_err(&[
        ("pkg.sv", "package p;\n  typedef enum logic [1:0] { CONFIG = 2'b00, B = 2'b01 } e_t;\nendpackage\n"),
        ("bfm.sv", "module m2;\n  int CONFIG = 5;\n  int CONFIG = 6;\nendmodule\n"),
    ]);
    assert!(err.contains("duplicate declaration of 'CONFIG'"), "got: {err}");
    assert!(err.contains("bfm.sv:3"), "must point at the second local decl; got: {err}");
    assert!(
        !err.contains("enum member"),
        "must blame the local declaration, not the package member; got: {err}"
    );
}
