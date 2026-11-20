//! Built-in net arrays resolve drivers per unpacked element (IEEE 1800-2023
//! §6.6.1, §6.6.3, §6.6.5, §7.4.2, and §23.3.3).

use xezim::simulate;

const SOURCE: &str = r#"
module source #(parameter V = 1)(output wire o);
  assign o = 1'(V);
endmodule
"#;

const INOUT_SOURCE: &str = r#"
module source(output wire o);
  assign o = 1'b1;
endmodule

module bidirectional(inout wire p);
  assign p = 1'b0;
endmodule
"#;

fn check(modules: &str, body: &str, sample: &str, expected: &str) {
    let src = format!(
        r#"`timescale 1ns/1ps
{modules}
module top;
  {body}
  initial begin
    #1;
    $display("value=%b", {sample});
    $finish;
  end
endmodule
"#
    );
    let sim = simulate(&src, 2_000).expect("simulate net driver test");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|entry| entry.message.trim().strip_prefix("value="))
        .collect();
    assert_eq!(actual, [expected]);
}

#[test]
fn two_outputs_resolve_on_one_wire_element() {
    check(
        SOURCE,
        "wire a[1]; source s(a[0]); source #(0) t(a[0]);",
        "a[0]",
        "x",
    );
}

#[test]
fn two_continuous_assignments_resolve_on_one_wire_element() {
    check(
        "",
        "wire a[1]; assign a[0] = 1'b1; assign a[0] = 1'b0;",
        "a[0]",
        "x",
    );
}

#[test]
fn wor_resolves_each_element() {
    check(
        SOURCE,
        "wor a[1], b[1]; source #(0) s(a[0]); source t(a[0]); source u(b[0]); source #(0) v(b[0]);",
        "{a[0], b[0]}",
        "11",
    );
}

#[test]
fn wand_resolves_each_element() {
    check(
        SOURCE,
        "wand a[1], b[1]; source s(a[0]); source #(0) t(a[0]); source #(0) u(b[0]); source v(b[0]);",
        "{a[0], b[0]}",
        "00",
    );
}

#[test]
fn inout_element_connection_resolves_on_both_sides() {
    check(
        INOUT_SOURCE,
        "wire a[1]; source s(a[0]); bidirectional t(a[0]);",
        "{a[0], t.p}",
        "xx",
    );
}

#[test]
fn whole_array_outputs_resolve_on_the_same_element() {
    check(
        r#"
module source #(parameter V = 1)(output wire o[1]);
  assign o[0] = 1'(V);
endmodule
"#,
        "wire a[1]; source s(a); source #(0) t(a);",
        "a[0]",
        "x",
    );
}

#[test]
fn tri0_elements_pull_low_with_a_z_drive_or_no_drive() {
    check("", "tri0 a[2]; assign a[0] = 1'bz;", "{a[0], a[1]}", "00");
}

#[test]
fn packed_bit_drivers_leave_the_other_bit_at_z() {
    check(
        SOURCE,
        "wire [1:0] a; source s(a[0]); source #(0) t(a[0]);",
        "a",
        "zx",
    );
}

#[test]
fn scalar_wire_resolves_two_outputs() {
    check(SOURCE, "wire a; source s(a); source #(0) t(a);", "a", "x");
}

#[test]
fn scalar_inout_connection_resolves_on_both_sides() {
    check(
        INOUT_SOURCE,
        "wire a; source s(a); bidirectional t(a);",
        "{a, t.p}",
        "xx",
    );
}

#[test]
fn drivers_on_different_elements_stay_independent() {
    check(
        SOURCE,
        "wire a[2]; source s(a[0]); source #(0) t(a[1]);",
        "{a[0], a[1]}",
        "10",
    );
}

#[test]
fn one_output_drives_one_element() {
    check(SOURCE, "wire a[1]; source s(a[0]);", "a[0]", "1");
}

#[test]
fn agreeing_outputs_keep_the_common_value() {
    check(
        SOURCE,
        "wire a[1]; source s(a[0]); source t(a[0]);",
        "a[0]",
        "1",
    );
}
