//! IEEE 1800-2023 §20.6.1(f): nested aggregate typedefs keep the scope
//! of their definitions in `$typename` strings.

use xezim::simulate;

fn check_typenames(src: &str, expected: &[&str]) {
    let sim = simulate(src, 100).expect("simulate failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .flat_map(|o| o.message.lines())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn typename_nested_package_types_on_nets_and_variables() {
    const SRC: &str = r#"package tn_pkg;
  typedef enum int { OFF, ON } mode_e;
  typedef struct { real v; shortreal sr; } sub_s;
  typedef struct { sub_s sub; mode_e mode; } deep_s;
  nettype deep_s deep_net;

  typedef struct packed { bit [3:0] v; } packed_s;
  typedef union packed { packed_s s; bit [3:0] raw; } payload_u;
  typedef struct packed { payload_u payload; mode_e mode; } packet_s;
  nettype packet_s packet_net;
endpackage

module top;
  import tn_pkg::*;
  deep_net dn;
  packet_net pn;
  tn_pkg::deep_s dv;
  tn_pkg::packet_s pv;
  initial begin
    $display("T|dn=%s", $typename(dn));
    $display("T|dv=%s", $typename(dv));
    $display("T|pn=%s", $typename(pn));
    $display("T|pv=%s", $typename(pv));
  end
endmodule
"#;
    const DEEP: &str = concat!(
        "struct{struct{real v;shortreal sr;}tn_pkg::sub_s sub;",
        "enum{OFF=32'sd0,ON=32'sd1}tn_pkg::mode_e mode;}tn_pkg::deep_s",
    );
    const PACKET: &str = concat!(
        "struct packed{union packed{struct packed{bit[3:0] v;}tn_pkg::packed_s s;",
        "bit[3:0] raw;}tn_pkg::payload_u payload;",
        "enum{OFF=32'sd0,ON=32'sd1}tn_pkg::mode_e mode;}tn_pkg::packet_s",
    );
    check_typenames(
        SRC,
        &[
            &format!("T|dn={DEEP}"),
            &format!("T|dv={DEEP}"),
            &format!("T|pn={PACKET}"),
            &format!("T|pv={PACKET}"),
        ],
    );
}

#[test]
fn typename_nested_local_types_keep_instance_scope() {
    const SRC: &str = r#"package a;
  typedef struct { real v; } pkg_s;
endpackage

module child;
  typedef struct { real v; } local_s;
  typedef struct { local_s s; } child_s;
  child_s nested;
  initial #1 $display("T|child=%s", $typename(nested));
endmodule

module top;
  child sub();
  typedef struct { real v; } local_sub_s;
  typedef enum bit { IDLE, RUNNING } local_e;
  typedef struct { local_sub_s s; local_e mode; } local_deep_s;
  typedef struct { real v; } \a::b ;
  typedef struct { \a::b s; } escaped_s;
  local_deep_s nested;
  escaped_s escaped_var;
  initial begin
    $display("T|local=%s", $typename(nested));
    $display("T|escaped=%s", $typename(escaped_var));
  end
endmodule
"#;
    check_typenames(
        SRC,
        &[
            concat!(
                "T|local=struct{struct{real v;}top.local_sub_s s;",
                "enum{IDLE=1'd0,RUNNING=1'd1}top.local_e mode;}top.local_deep_s",
            ),
            "T|escaped=struct{struct{real v;}top.a::b s;}top.escaped_s",
            "T|child=struct{struct{real v;}top.sub.local_s s;}top.sub.child_s",
        ],
    );
}

#[test]
fn typename_nested_class_types_keep_class_scope() {
    const SRC: &str = r#"class C;
  typedef struct packed { bit [3:0] v; } item_s;
endclass

class D;
  typedef struct packed { bit [7:0] v; } item_s;
endclass

module top;
  typedef struct packed { C::item_s c; D::item_s d; } pair_s;
  pair_s pair;
  initial $display("T|pair=%s", $typename(pair));
endmodule
"#;
    check_typenames(
        SRC,
        &[concat!(
            "T|pair=struct packed{struct packed{bit[3:0] v;}C::item_s c;",
            "struct packed{bit[7:0] v;}D::item_s d;}top.pair_s",
        )],
    );
}
