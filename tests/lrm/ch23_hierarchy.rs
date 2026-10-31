//! IEEE 1800-2023 clause 23: modules and hierarchy.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §23.10.1: defparam: relative, absolute, into generate scopes
#[test]
fn c23_10_1_defparam() {
    check(
        "p23g",
        include_str!("sv/p23g.sv"),
        "t23g",
        Order::Exact,
        &[
            "T|t23g.m1.l|P=7 Q=8",
            "T|t23g.m1.l2|P=40 Q=41",
            "T|t23g.m2.l|P=8 Q=9",
            "T|t23g.m2.l2|P=40 Q=41",
            "T|t23g.m2.g.gl|P=9 Q=90",
            "T|t23g.prec|P=33 Q=34",
            "T|t23g.dep|P=100 Q=101",
        ],
        &[],
    );
}

// §23.11, §23.2.4: bind to module types, $unit items
#[test]
fn c23_11_bind_and_unit_scope() {
    // Known gap (§23.11): `23.11a`: 3 reference lines differ from xezim's 3, e.g. reference `t23i.u2.c_path P=8 a=22`, xezim `t23i.u2.c_list P=3 a=22`
    // `23.11b` leaves out `u2.c_path.cnt`: xezim does not apply that bind
    // (the `23.11a` gap), so the name does not resolve, and a hierarchical
    // name that does not resolve is an elaboration error (§23.8).
    // Known gap (§23.11): `23.11c` reference `11 z3`, xezim `11 03`
    // The bound instances print from their own initial blocks at time 0; IEEE 1800 §4.7 leaves their order open.
    check(
        "p23i",
        include_str!("sv/p23i.sv"),
        "t23i",
        Order::Multiset,
        &[
            "T|23.11a|t23i.u1.c_all P=1 a=11",
            "T|23.11a|t23i.u1.c_list P=3 a=11",
            "T|23.11a|t23i.u2.c_all P=1 a=22",
            "T|23.11a|t23i.u2.c_in P=5 a=22",
            "T|26.unit|t23i.u1 2 1 501",
            "T|26.unit|t23i.u2 2 1 501",
            "T|26.unit|t23i.u3 2 1 501",
            "T|23.5|eo=30",
            "T|26.unit2|1 6 502",
        ],
        &[
            "T|23.11a|t23i.u2.c_path P=8 a=22",
            "T|23.11a|t23i.u2.c_list P=3 a=22",
            "T|23.11a|t23i.u3.c_",
            "T|23.11b|",
            "T|23.11c|",
        ],
    );
}

// §23.11: bind to a list of instances of a module type (bind tgt: u1, u3 chk ...)
#[test]
fn c23_11_bind_instance_list_and_path() {
    // Known gap (§23.11): `bind`: 2 reference lines differ from xezim's 2, e.g. reference `r_bind.u2.c_path P=8 a=22`, xezim `r_bind.u2.c_list P=1 a=22`
    check(
        "23.11_bind_instance_list_and_path",
        include_str!("sv/23.11_bind_instance_list_and_path.sv"),
        "r_bind",
        Order::Exact,
        &[
            "T|bind|r_bind.u1.c_list P=1 a=11",
            "T|bind|r_bind.u3.c_list P=1 a=33",
        ],
        &["T|bind|r_bind.u2.c_", "T|bind|r_bind.m.deep.c_"],
    );
}

// §23.2.2.1, §23.2.2: non-ANSI ports: implicit nets, port expressions
#[test]
fn c23_2_2_1_non_ansi_port_forms() {
    // Known gap (§23.2.2.1): `23.2.2.1e` reference `o5=110000 q5=5 s5=1 bits=8`, xezim `o5=110000 q5=x s5=0 bits=8`
    check(
        "p23a",
        include_str!("sv/p23a.sv"),
        "t23a",
        Order::Exact,
        &[
            "T|23.2.2.1a|c=17 d=1",
            "T|23.2.2.1b|y=c3 y2=c3",
            "T|23.2.2.1c|r=0110",
            "T|23.2.2.1d|o4=0101",
            "T|23.2.2.1f|b6=8",
            "T|23.2.2.1g|c=22 d=0",
        ],
        &["T|23.2.2.1e|"],
    );
}

// §23.2.2.1: non-ANSI port declarations
#[test]
fn c23_2_2_1_non_ansi_ports() {
    check(
        "r_nansi",
        include_str!("sv/r_nansi.sv"),
        "rna",
        Order::Exact,
        &["T|r1|y1=c y2=c i2=1 y3=c"],
        &[],
    );
}

// §23.2.1, §23.2.2: non-ANSI output declared reg, ANSI equivalent
#[test]
fn c23_2_2_1_non_ansi_reg_output() {
    check(
        "r_nansi2",
        include_str!("sv/r_nansi2.sv"),
        "rna2",
        Order::Exact,
        &["T|r1|init-decl: nonansi=c ansi=c  init-proc: nonansi=c"],
        &[],
    );
}

// §23.2.2.2, §23.3.3: ANSI ports: inherited direction/type, ref ports
#[test]
fn c23_2_2_2_ansi_port_forms() {
    // Known gap (§23.2.2.2): `23.2.2.2b` reference `us=10,11 ro=2.500000 oarr=2,1 so=hey!`, xezim `us=0,0 ro=2.500000 oarr=2,1 so=hey!`
    // Known gap (§23.2.2.3): `23.2.2.3a` reference `c=1 d=1 e=40 f=11`, xezim `c=1 d=1 e=40 f=42`
    // Known gap (§23.2.2.4): `23.2.2.4` reference `o1=710 o2=210 o3=0 o4=310`, xezim `o1=710 o2=210 o3=710 o4=310`
    check(
        "p23b",
        include_str!("sv/p23b.sv"),
        "t23b",
        Order::Exact,
        &[
            "T|23.2.2.2a|s=17 sum=-7 r=9 w=6",
            "T|23.3.3.ref|cnt=105 q='{'h77}",
        ],
        &["T|23.2.2.2b|", "T|23.2.2.3a|", "T|23.2.2.4|"],
    );
}

// §23.2.2.2: Signed input port connected to an unsigned actual (input signed [7:0] si; .si(uv) or .si(8'h80))
#[test]
fn c23_2_2_signed_input_port_unsigned_actual() {
    // Known gap (§23.2.2.2): `sx1` reference `ff80 1 ffc0`, xezim `0080 0 0040`
    // Known gap (§23.2.2.2): `sx2` reference `ff80 1 ffc0`, xezim `0080 0 0040`
    // Known gap (§23.2.2.2): `sx4` reference `ff80`, xezim `0080`
    // Known gap (§23.2.2.2): `sx5` reference `ff81 1 ffc0 ff81`, xezim `0081 0 0040 0081`
    check(
        "23.2.2_signed_input_port_unsigned_actual",
        include_str!("sv/23.2.2_signed_input_port_unsigned_actual.sv"),
        "r_sx",
        Order::Exact,
        &["T|sx3|ff80 1"],
        &["T|sx1|", "T|sx2|", "T|sx4|", "T|sx5|"],
    );
}

// §23.2.3, §23.10.2.1: parameterized modules, positional overrides
#[test]
fn c23_2_3_parameterized_modules() {
    // Known gap (§23.2.3): `t23f.u0` reference `ARR='{1, 2, 3} STR=def PS=12 unb=1`, xezim `ARR=x STR=def PS=12 unb=1`
    // Known gap (§23.2.3): `t23f.u1`: 2 reference lines differ from xezim's 2, e.g. reference `P=255 bitsP=8 Q=15 S=-1 I=4 R=2.000000`, xezim `P=255 bitsP=8 Q=255 S=255 I=3.7 R=2.000000`
    // Known gap (§23.2.3): `t23f.u2` reference `ARR='{7, 8, 9} STR=xyz PS=5a unb=0`, xezim `ARR=0 STR=xyz PS=5a unb=0`
    // Known gap (§23.2.3): `t23f.u3` reference `ARR='{1, 2, 3} STR=def PS=12 unb=1`, xezim `ARR=x STR=def PS=12 unb=1`
    // Known gap (§23.2.3): `t23f.u4` reference `ARR='{1, 2, 3} STR=def PS=12 unb=1`, xezim `ARR=x STR=def PS=12 unb=1`
    check(
        "p23f",
        include_str!("sv/p23f.sv"),
        "t23f",
        Order::Exact,
        &[
            "T|t23f.u0|P=4 bitsP=32 Q=1 S=0 I=0 R=1.000000",
            "T|t23f.u0|A=2 B=6 LB=7 BODY=9",
            "T|t23f.u0|bitsT=32 tv=-1 bitsT2=32 t2v=-1",
            "T|t23f.u1|A=5 B=15 LB=16 BODY=9",
            "T|t23f.u1|bitsT=32 tv=-1 bitsT2=32 t2v=-1",
            "T|t23f.u2|P=4 bitsP=32 Q=1 S=0 I=0 R=1.000000",
            "T|t23f.u2|A=5 B=1 LB=2 BODY=9",
            "T|t23f.u2|bitsT=12 tv=4095 bitsT2=12 t2v=4095",
            "T|t23f.u3|P=4 bitsP=32 Q=1 S=0 I=0 R=1.000000",
            "T|t23f.u3|A=2 B=6 LB=7 BODY=9",
            "T|t23f.u3|bitsT=8 tv=255 bitsT2=16 t2v=-1",
            "T|t23f.u4|P=10 bitsP=32 Q=11 S=0 I=0 R=1.000000",
            "T|t23f.u4|A=2 B=6 LB=7 BODY=9",
            "T|t23f.u4|bitsT=32 tv=-1 bitsT2=32 t2v=-1",
            "T|t23f.s0|P=hello bits=40",
            "T|t23f.r0|P=2.500000 1",
            "T|t23f.g0|P=-3 lt=1 bits=32",
            "T|t23f.g1|P=-8 lt=1 bits=4",
            "T|t23f.g2|P=7 lt=0 bits=3",
        ],
        &[
            "T|t23f.u0|ARR=",
            "T|t23f.u1|P=255 bitsP=8 Q=",
            "T|t23f.u1|ARR=",
            "T|t23f.u2|ARR=",
            "T|t23f.u3|ARR=",
            "T|t23f.u4|ARR=",
        ],
    );
}

// §23.3.1: several uninstantiated modules elaborate as tops; $root references
#[test]
fn c23_3_1_several_top_modules() {
    // The two `%m` lines come from two top-level instances at the same time; IEEE 1800 §4.7 leaves their order open.
    check(
        "p23top",
        include_str!("sv/p23top.sv"),
        "",
        Order::Multiset,
        &[
            "T|23.3.1|topA.s",
            "T|23.3.1|topB.s",
            "T|23.3.1a|A sees B: 5",
            "T|23.3.1b|B up=5",
        ],
        &[],
    );
}

// §23.3.2, §23.3.3: implicit nets from connections, positional blanks, coercion
#[test]
fn c23_3_2_port_connections() {
    // Deliberate divergence (§23.3.3.7): `wide`: the input port zero-extends its narrow actual (bits 7:4); the reference leaves them z
    // Known gap (§23.3.3.7): in the same line, the net bits beyond the 8-bit output port read 0;
    // the reference leaves them undriven (z).
    // Known gap (§23.3.3): `23.3.3c` reference `sx16=zzfd`, xezim `sx16=fffd`
    // Known gap (§23.3.3): `23.3.3d` reference `ext=ff80`, xezim `ext=0080`
    check(
        "p23c",
        include_str!("sv/p23c.sv"),
        "t23c",
        Order::Exact,
        &[
            "T|23.3.3a|imp=0 bits=1",
            "T|23.3.3e|dio=5 dio_o=5",
            "T|23.3.3f|undriven=zzzz",
            "T|23.3.3g|bus=zzzz",
            "T|23.3.3h|bus=0011",
            "T|23.3.3i|bus=xxxx",
            "T|23.3.3j|bus=1100 io=1100",
            "T|23.3.2a|po=44",
        ],
        &["T|23.3.3b|", "T|23.3.3c|", "T|23.3.3d|"],
    );
}

// §23.3.3.2: output port connected to a parent variable
#[test]
fn c23_3_3_2_output_to_variable() {
    // Known gap (§23.3.3.1): `23.3.3.1` reference `w=x r=000x`, xezim `w=x r=0000`
    check(
        "p23k",
        include_str!("sv/p23k.sv"),
        "t23k",
        Order::Exact,
        &["T|23.3.3.2|pv=5"],
        &["T|23.3.3.1|"],
    );
}

// §23.3.3.3: wand/wor/tri0/tri1 nets across ports
#[test]
fn c23_3_3_3_net_kinds_across_ports() {
    // Not compared (§23.3.3.4): the reference delivers z through the interconnect; xezim delivers the driven value, as §6.6.8 describes
    // Known gap (§23.3.3.7): `23.3.3.7c` reference `wor_port=1101 inner=1101`, xezim `wor_port=xx0x inner=0101`
    check(
        "p23d",
        include_str!("sv/p23d.sv"),
        "t23d",
        Order::Exact,
        &[
            "T|23.3.3.7a|wand=1000 wor=1111",
            "T|23.3.3.7b|tri1=1111 tri0=0000",
            "T|23.3.3.7d|xo=0000 yo=1",
        ],
        &["T|23.3.3.4|", "T|23.3.3.7c|"],
    );
}

// §23.3.3.4: interconnect nets and ports
#[test]
fn c23_3_3_4_interconnect_ports() {
    // Known gap (§23.3.3.4): `ic3` reference `ico3=zzz1`, xezim `ico3=1001`
    check(
        "r_ic",
        include_str!("sv/r_ic.sv"),
        "r_ic",
        Order::Exact,
        &["T|ic|ico=9 ico2=6"],
        &["T|ic3|"],
    );
}

// §23.3.3.5: instance arrays: vector split, replication, array actuals
#[test]
fn c23_3_3_5_instance_arrays() {
    // Known gap (§23.3.3.5): `23.3.3.5g` reference `u3.a=4 u0.a=1 v0.a=4 v3.a=1`, xezim `u3.a=4 u0.a=1 v0.a=1 v3.a=4`
    check(
        "p23e",
        include_str!("sv/p23e.sv"),
        "t23e",
        Order::Exact,
        &[
            "T|23.3.3.5a|ys=5432 zs=0000",
            "T|23.3.3.5b|ys2=6543 zs2=0000",
            "T|23.3.3.5c|ob=01011001",
            "T|23.3.3.5d|uy='{'h2, 'h3, 'h4, 'h5} uz=0000",
            "T|23.3.3.5e|ys3=51 id1=5 id0=1",
            "T|23.3.3.5f|oy=44 oz=00 nz5=3 nz4.y=4",
            "T|23.3.3.5h|16",
        ],
        &["T|23.3.3.5g|"],
    );
}

// §23.3.3: Port connected to a constant (1'b0 / 1'b1): edges at time 0
#[test]
fn c23_3_3_const_port_edges() {
    // Known gap (§23.3.3): `r1` reference `tie0: pe=0 ne=1  tie1: pe=1 ne=0`, xezim `tie0: pe=1 ne=1  tie1: pe=1 ne=1`
    // Known gap (§23.3.3): `r0`: xezim prints extra `rcp2.e0 posedge at t=0 clk=0`
    check(
        "23.3.3_const_port_edges",
        include_str!("sv/23.3.3_const_port_edges.sv"),
        "rcp2",
        Order::Exact,
        &["T|r0|rcp2.e1 posedge at t=0 clk=1"],
        &["T|r1|", "T|r0|rcp2.e0 posedge at t=0 clk=0"],
    );
}

// §23.3.3: variable driven by an output port and procedurally
#[test]
fn c23_3_3_variable_port_driver() {
    check(
        "r_vardrv",
        include_str!("sv/r_vardrv.sv"),
        "r_vardrv",
        Order::Exact,
        &["T|vd|pv=9"],
        &[],
    );
}

// §23.3.2, §23.4, §23.10.4.1: nested modules, several instances per statement, elaboration order
#[test]
fn c23_4_nested_modules() {
    check(
        "p23j",
        include_str!("sv/p23j.sv"),
        "t23j",
        Order::Exact,
        &[
            "T|23.4|nested sees outer: 74",
            "T|23.4|nested sees outer: 74",
            "T|23.10.4|o=0 0 5 gen=11",
        ],
        &[],
    );
}

// §23.6, §23.7, §23.8, §23.9: hierarchical names, upward references, $root
#[test]
fn c23_6_hierarchical_names() {
    check(
        "p23h",
        include_str!("sv/p23h.sv"),
        "t23h",
        Order::Exact,
        &[
            "T|23.6a|1 1 1 3",
            "T|23.6b|mirror=3 get=1",
            "T|23.6c|seen=1",
            "T|23.6d|t=3 hits=1",
            "T|23.6e|nb=77",
            "T|23.6f|if=6 esc=5",
            "T|23.7|w2=55",
            "T|23.8a|t23h.w.inst_x.ul by_type=42 by_inst=42 top=9",
            "T|23.6g|mirror=c",
            "T|23.9a|t23h.w.inst_x.ul helper=2042",
            "T|23.6h|mirror=3",
        ],
        &[],
    );
}

// §23.2.2, §23.3.2, §23.4, §23.8, §23.10.1, §23.10.2, ...: ports, instances, nested modules, upward names, defparam, bind
#[test]
fn c23_hierarchy() {
    // Deliberate divergence (§23.3.3.7): `big12`: the input port zero-extends its narrow actual (bits 7:4); the reference leaves them z
    // Known gap (§23.3.3.7): in the same line, the net bits beyond the 8-bit output port read 0;
    // the reference leaves them undriven (z).
    // Known gap (§23.6): `23.6c` reference `2`, xezim `0`
    check(
        "c23_hier",
        include_str!("sv/c23_hier.sv"),
        "c23",
        Order::Exact,
        &[
            "T|23.3.3|ub=zzzz",
            "T|23.3.3|ub=0001",
            "T|23.10.1|c23.d0 X=1 Y=2",
            "T|23.10.1|c23.d1 X=5 Y=2",
            "T|23.10.1|c23.d2 X=1 Y=7",
            "T|23.10.1|c23.d3 X=3 Y=4",
            "T|23.10.1|c23.d4 X=100 Y=200",
            "T|23.2.2a|y1=c in1=1",
            "T|23.2.2b|y2=8 tv=8 po=5",
            "T|23.3.2|o=7 o2=7",
            "T|23.6a|11 22",
            "T|23.8b|leaf task called",
            "T|23.6b|99",
            "T|23.6d|4",
            "T|23.8a|up=22",
            "T|23.4|nested=33",
            "T|23.11|bound sees 9 in c23.bt.bm",
        ],
        &["T|23.3.3b|", "T|23.6c|"],
    );
}

// §13.7, §23.3.3, §23.6, §23.9: port coercion, hierarchical names, upward function resolution
#[test]
fn c23_hierarchy_more() {
    // Known gap (§23.6): `23.6e` reference `5 30`, xezim `5 1`
    // Known gap (§23.6): `23.6h` reference `30`, xezim `1`
    check(
        "c23_more",
        include_str!("sv/c23_more.sv"),
        "c23x",
        Order::Exact,
        &[
            "T|23.6f|15",
            "T|23.6g|4 30",
            "T|23.9a|4",
            "T|23.3.3c|zzzz",
            "T|23.3.3d|3",
            "T|23.3.3e|xxxx",
            "T|23.10|32",
            "T|23.6i|16",
        ],
        &["T|23.6e|", "T|23.6h|"],
    );
}

// §23.11, §6.10: a bind port actual the target does not declare is an implicit
// net in the target, not the bind scope's signal of that name
#[test]
fn c23_11_bind_connection_implicit_net() {
    // The bound instances print from their own initial blocks at time 30; IEEE 1800 §4.7 leaves their order open.
    check(
        "23.11_bind_connection_implicit_net",
        include_str!("sv/23.11_bind_connection_implicit_net.sv"),
        "bcin",
        Order::Multiset,
        &[
            "T|23.11|bcin.bare.mon edges=0",
            "T|23.11|bcin.own.mon edges=1",
            "T|23.11|bcin.far.mon edges=0",
        ],
        &[],
    );
}
