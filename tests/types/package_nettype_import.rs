//! IEEE 1800-2023 §26.3 / §6.6.7: an explicitly imported nettype is the
//! package's declaration, including its data type and resolver binding.
//! The programs use §3.12.1 compilation-unit imports and §26.4 header imports.

fn check(src: &str, expected: &[&str]) {
    let result = xezim::simulate(src, 100).expect("package nettype import must elaborate");
    let lines: Vec<_> = result.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(lines, expected);
}

#[test]
fn module_unresolved() {
    check(
        include_str!("package_nettype_import/module_unresolved.sv"),
        &["NET 1.50", "NET -3.25"],
    );
}

#[test]
fn module_resolved() {
    check(
        include_str!("package_nettype_import/module_resolved.sv"),
        &["NET 3.75", "NET -2.75", "LOCAL 99.00"],
    );
}

#[test]
fn unit_unresolved() {
    check(
        include_str!("package_nettype_import/unit_unresolved.sv"),
        &["NET 1.50", "NET -3.25"],
    );
}

#[test]
fn unit_resolved() {
    check(
        include_str!("package_nettype_import/unit_resolved.sv"),
        &["NET 3.75", "NET -2.75", "LOCAL 99.00"],
    );
}

#[test]
fn header_unresolved() {
    check(
        include_str!("package_nettype_import/header_unresolved.sv"),
        &["PORT 1.50 1.50", "PORT -3.25 -3.25"],
    );
}

#[test]
fn header_resolved() {
    check(
        include_str!("package_nettype_import/header_resolved.sv"),
        &["PORT 3.75 3.75", "PORT -2.75 -2.75"],
    );
}

#[test]
fn collision_nettype() {
    check(
        include_str!("package_nettype_import/collision_nettype.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

#[test]
fn collision_typedef() {
    check(
        include_str!("package_nettype_import/collision_typedef.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

#[test]
fn wildcard_unresolved() {
    check(
        include_str!("package_nettype_import/wildcard_unresolved.sv"),
        &["NET 1.50", "NET -3.25"],
    );
}

#[test]
fn wildcard_resolved() {
    check(
        include_str!("package_nettype_import/wildcard_resolved.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

#[test]
fn other_items() {
    check(
        include_str!("package_nettype_import/other_items.sv"),
        &["ITEMS 15 8", "ITEMS 249 8"],
    );
}

#[test]
fn unit_collision_nettype() {
    check(
        include_str!("package_nettype_import/unit_collision_nettype.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

#[test]
fn unit_collision_typedef() {
    check(
        include_str!("package_nettype_import/unit_collision_typedef.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

/// §26.3: a resolver imported into the nettype's package keeps working.
#[test]
fn wildcard_imported_resolver() {
    check(
        include_str!("package_nettype_import/wildcard_imported_resolver.sv"),
        &["NET 3.75", "NET -2.75"],
    );
}

#[test]
fn wildcard_local_resolver() {
    check(
        include_str!("package_nettype_import/wildcard_local_resolver.sv"),
        &["NET 3.75", "NET -2.75", "LOCAL 99.00"],
    );
}
