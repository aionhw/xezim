//! IEEE 1800-2023 §23.3.3.8: "Any expressions on a port shall be treated as
//! any other expression in an assignment." The follow-up to #1324 covers the
//! actuals that are not literals: nets, variables, parameters, package
//! constants and expressions bound to 2-state (§6.11.2), pull (§6.6.5),
//! charge-storage (§6.6.4), real (§6.12.1) and narrower formals (§10.7).

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000).expect("simulate");
    sim.output.iter().map(|line| line.message.clone()).collect()
}

fn check(src: &str, expected: &[&str]) {
    let out = messages(src);
    for line in expected {
        assert!(out.iter().any(|m| m == line), "missing {line:?}: {out:#?}");
    }
}

/// §6.11.2: a 2-state formal maps X and Z to 0 whatever its actual, whether
/// the formal is substituted or keeps its own storage through a connection
/// assign (§23.3.3.2), for direct, typedef, packed struct and union, offset
/// and ascending ranges, `int`, implicit-kind and non-ANSI ports. A 2-state
/// output port drops the X and Z its body drives.
#[test]
fn two_state_formals_drop_x_and_z_of_every_actual() {
    let src = r#"
`timescale 1ns/1ps
package pk;
  localparam logic [3:0] PX = 4'b1x01;
endpackage
typedef bit [3:0] b4_t;
typedef struct packed { bit [3:0] h, l; } s_t;
typedef union packed { bit [3:0] a; bit [3:0] b; } u_t;
module tb4(input var bit [3:0] b);  initial #1 $display("%m b=%b", b); endmodule
module tt4(input var b4_t b);       initial #1 $display("%m b=%b", b); endmodule
module tb8(input var bit [7:0] b);  initial #1 $display("%m b=%b", b); endmodule
module tb41(input var bit [4:1] b); initial #1 $display("%m b=%b", b); endmodule
module tb03(input var bit [0:3] b); initial #1 $display("%m b=%b", b); endmodule
module tb22(input var bit [1:0][1:0] b); initial #1 $display("%m b=%b", b); endmodule
module tun(input var u_t b); initial #1 $display("%m b=%b", b); endmodule
module tst(input var s_t b); initial #1 $display("%m b=%b", b); endmodule
module tint(input var int b); initial #1 $display("%m b=%b", b[3:0]); endmodule
module tkind(input bit [3:0] b); initial #1 $display("%m b=%b", b); endmodule
module tnet_int(input int b); initial #1 $display("%m b=%b", b[3:0]); endmodule
module tscalar(input bit b); initial #1 $display("%m b=%b", b); endmodule
module tnonansi(b);
  input bit [3:0] b;
  initial #1 $display("%m b=%b", b);
endmodule
module tout(output bit [3:0] o); assign o = 4'b1x0z; endmodule
module toutv(output var bit [3:0] o); initial o = 4'b1x0z; endmodule
module top;
  wire [3:0] w = 4'b1x01;
  logic [3:0] v = 4'b1x01;
  logic x1 = 1'bx;
  parameter logic [3:0] P = 4'b1x01;
  localparam logic [3:0] L = 4'b1x01;
  wire [3:0] ob, obv;
  tb4 a_wire(w);
  tb4 a_var(v);
  tb4 a_param(P);
  tb4 a_lparam(L);
  tb4 a_pkg(pk::PX);
  tb4 a_signed($signed(4'b1x01));
  tb4 a_concat({v});
  tb4 a_cast(4'(4'b1x01));
  tb4 a_cond(1 ? 4'b1x01 : 4'b0);
  tt4 a_typedef(P);
  tb8 c_narrow(4'b1x0z);
  tb8 d_wide(16'bzzzz_zzzz_1x0z_1x0z);
  tb4 e_x('x);
  tt4 e_typedef_z('z);
  tint e_int('x);
  tb41 f_offset(v);
  tb03 f_ascending(4'b1x01);
  tb22 f_packed_2d(v);
  tun f_union(4'b1x01);
  tst g_struct(8'bx0x1x0x1);
  tkind k_wire(w);
  tkind k_var(v);
  tnet_int k_int(w);
  tscalar k_scalar(x1);
  tnonansi k_nonansi(v);
  tout o_net(ob);
  toutv o_var(obv);
  initial #1 $display("outputs %b %b", ob, obv);
endmodule
"#;
    check(
        src,
        &[
            "top.a_wire b=1001",
            "top.a_var b=1001",
            "top.a_param b=1001",
            "top.a_lparam b=1001",
            "top.a_pkg b=1001",
            "top.a_signed b=1001",
            "top.a_concat b=1001",
            "top.a_cast b=1001",
            "top.a_cond b=1001",
            "top.a_typedef b=1001",
            "top.c_narrow b=00001000",
            "top.d_wide b=10001000",
            "top.e_x b=0000",
            "top.e_typedef_z b=0000",
            "top.e_int b=0000",
            "top.f_offset b=1001",
            "top.f_ascending b=1001",
            "top.f_packed_2d b=1001",
            "top.f_union b=1001",
            "top.g_struct b=00010001",
            "top.k_wire b=1001",
            "top.k_var b=1001",
            "top.k_int b=1001",
            "top.k_scalar b=0",
            "top.k_nonansi b=1001",
            "outputs 1000 1000",
        ],
    );
}

/// §6.6.5: a tri0 or tri1 input pulls the Z bits of any actual, not only of
/// a literal. §6.6.4: a trireg input holds its charge, initially X, on the
/// bits its actual leaves at Z.
#[test]
fn pull_and_charge_inputs_resolve_z_of_every_actual() {
    let src = r#"
`timescale 1ns/1ps
package pk;
  localparam logic [3:0] PZ = 4'b1z0z;
  logic [3:0] pv = 4'b1z0z;
endpackage
module tp1(input tri1 [3:0] b); initial #1 $display("%m b=%b", b); endmodule
module tp0(input tri0 [3:0] b); initial #1 $display("%m b=%b", b); endmodule
module ttr(input trireg [3:0] b); initial #1 $display("%m b=%b", b); endmodule
module hp #(parameter logic [3:0] HP = 4'b1z0z) ();
  tp1 h1(HP);
  tp0 h0(HP);
endmodule
module top;
  logic [3:0] vz = 4'b1z0z;
  reg [7:0] rz = 8'b1z0z1z0z;
  localparam logic [3:0] LZ = 4'b1z0z;
  wire [3:0] wz;
  wire [3:0] wd = 4'b1z0z;
  tp1 var1(vz);
  tp0 var0(vz);
  tp1 part_select(rz[3:0]);
  tp1 pkg_var(pk::pv);
  tp1 lparam1(LZ);
  tp0 lparam0(LZ);
  hp header();
  tp1 pkg1(pk::PZ);
  tp0 pkg0(pk::PZ);
  tp1 concat1({4'b1z0z});
  tp0 concat0({2'b1z, 2'b0z});
  tp1 undriven(wz);
  tp1 driven(wd);
  ttr charge(4'b1z0z);
endmodule
"#;
    check(
        src,
        &[
            "top.var1 b=1101",
            "top.var0 b=1000",
            "top.part_select b=1101",
            "top.pkg_var b=1101",
            "top.lparam1 b=1101",
            "top.lparam0 b=1000",
            "top.header.h1 b=1101",
            "top.header.h0 b=1000",
            "top.pkg1 b=1101",
            "top.pkg0 b=1000",
            "top.concat1 b=1101",
            "top.concat0 b=1000",
            "top.undriven b=1111",
            "top.driven b=1101",
            "top.charge b=1x0x",
        ],
    );
}

/// §26.3, §10.7: a package constant on a narrower formal is truncated to the
/// formal's width. §6.12.1: a real package constant on an integral formal
/// rounds, and an integral variable on a real formal converts, also at the
/// real's own 64 bits, as any assignment does.
#[test]
fn package_and_real_actuals_take_the_formal_type() {
    let src = r#"
`timescale 1ns/1ps
package pk;
  localparam logic [11:0] PW = 12'h1ff;
  localparam bit [3:0] PB = 4'b1001;
  localparam real PR = 2.5;
endpackage
module tw8(input wire [7:0] b); initial #1 $display("%m bits=%0d b=%0d", $bits(b), b); endmodule
module tb4(input var bit [3:0] b); initial #1 $display("%m b=%b", b); endmodule
module tr(input real r); initial #1 $display("%m r=%f", r); endmodule
module top;
  logic [7:0] lv = 8'd200;
  logic [63:0] lw = 64'd300;
  tw8 pkg_wide(pk::PW);
  tb4 pkg_two_state(pk::PB);
  tw8 pkg_real(pk::PR);
  tr from_vector(lv);
  tr from_wide(lw);
endmodule
"#;
    check(
        src,
        &[
            "top.pkg_wide bits=8 b=255",
            "top.pkg_two_state b=1001",
            "top.pkg_real bits=8 b=3",
            "top.from_vector r=200.000000",
            "top.from_wide r=300.000000",
        ],
    );
}
